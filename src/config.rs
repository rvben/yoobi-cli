//! Resolve explicit environment overrides or an identity-bound saved login.
use crate::Error;
use serde_json::{Value, json};
use std::{env, fmt, fs};
use url::Url;

#[derive(Clone)]
pub struct Config {
    pub base_url: String,
    pub username: String,
    password: String,
}
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("base_url", &self.base_url)
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .finish()
    }
}
impl Config {
    pub fn new(base_url: &str, username: String, password: String) -> Result<Self, Error> {
        let base_url = normalize_base(base_url)?;
        if username.trim().is_empty() || password.is_empty() {
            return Err(Error::Auth {
                message: "Yoobi username and password are required".into(),
            });
        }
        Ok(Self {
            base_url,
            username,
            password,
        })
    }
    /// Explicit environment credentials override the saved login. Saved secrets are
    /// reused only when the effective origin and username still match their identity.
    pub fn load() -> Result<Self, Error> {
        let base = env::var("YOOBI_BASE_URL").ok();
        let user = env::var("YOOBI_USERNAME").ok();
        let password = environment_password()?;
        if let (Some(base), Some(user), Some(password)) = (&base, &user, &password) {
            return Self::new(base, user.clone(), password.clone());
        }
        let saved = crate::auth::read_saved(&crate::auth::config_dir()?)?;
        resolve(
            base.as_deref(),
            user.as_deref(),
            password,
            saved.as_ref(),
            &crate::auth::NativeCredentials,
        )
    }
    /// Compatibility alias for callers of the original environment-only loader.
    pub fn from_env() -> Result<Self, Error> {
        Self::load()
    }
    /// Configuration inspection never reads the OS credential store or makes requests.
    pub fn status() -> Value {
        let base = env::var("YOOBI_BASE_URL").ok();
        let user = env::var("YOOBI_USERNAME").ok();
        let password = match environment_password() {
            Ok(p) => p,
            Err(_) => return json!({"configured":false,"source":"environment","verified":false}),
        };
        if let (Some(base), Some(user), Some(password)) = (&base, &user, &password) {
            return json!({"configured":Self::new(base,user.clone(),password.clone()).is_ok(),"source":"environment","verified":false});
        }
        let saved = crate::auth::config_dir()
            .and_then(|dir| crate::auth::read_saved(&dir))
            .ok()
            .flatten();
        let effective_base = base
            .as_deref()
            .or(saved.as_ref().map(|s| s.base_url.as_str()));
        let effective_user = user
            .as_deref()
            .or(saved.as_ref().map(|s| s.username.as_str()));
        let identity_valid = effective_base
            .and_then(|b| normalize_base(b).ok())
            .is_some()
            && effective_user.is_some_and(|u| !u.trim().is_empty());
        let secret_valid = if let Some(p) = &password {
            !p.is_empty()
        } else {
            saved.as_ref().is_some_and(|s| {
                effective_base
                    .and_then(|b| normalize_base(b).ok())
                    .as_deref()
                    == Some(&s.base_url)
                    && effective_user == Some(&s.username)
            })
        };
        json!({"configured":identity_valid&&secret_valid,"source":if password.is_some() {"environment"} else if saved.is_some() {"keychain"} else {"none"},"saved":saved.is_some(),"verified":false})
    }
    pub(crate) fn password(&self) -> &str {
        &self.password
    }
}
pub(crate) fn environment_password() -> Result<Option<String>, Error> {
    match (
        env::var("YOOBI_PASSWORD").ok(),
        env::var_os("YOOBI_PASSWORD_FILE"),
    ) {
        (Some(_), Some(_)) => Err(Error::Usage {
            message: "set only one of YOOBI_PASSWORD and YOOBI_PASSWORD_FILE".into(),
        }),
        (Some(p), None) => Ok(Some(p)),
        (None, Some(path)) => {
            let metadata = fs::symlink_metadata(&path).map_err(|_| Error::Auth {
                message: "cannot read password file".into(),
            })?;
            if !metadata.file_type().is_file() {
                return Err(Error::Auth {
                    message: "password file must be a regular file, not a symlink".into(),
                });
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(Error::Auth {
                        message: "password file must be private (chmod 600)".into(),
                    });
                }
            }
            fs::read_to_string(path)
                .map(|p| Some(p.trim_end_matches(['\r', '\n']).to_owned()))
                .map_err(|_| Error::Auth {
                    message: "cannot read password file".into(),
                })
        }
        (None, None) => Ok(None),
    }
}
pub(crate) fn resolve(
    base: Option<&str>,
    user: Option<&str>,
    password: Option<String>,
    saved: Option<&crate::auth::SavedLogin>,
    store: &impl crate::auth::Credentials,
) -> Result<Config, Error> {
    let base = base
        .or(saved.map(|s| s.base_url.as_str()))
        .ok_or_else(|| Error::Auth {
            message: "no saved login; run `yoobi-cli auth login`".into(),
        })?;
    let user = user
        .or(saved.map(|s| s.username.as_str()))
        .ok_or_else(|| Error::Auth {
            message: "Yoobi username is required; run `yoobi-cli auth login`".into(),
        })?;
    let base = normalize_base(base)?;
    let password = match password {
        Some(p) => p,
        None => {
            let saved = saved.ok_or_else(|| Error::Auth {
                message: "Yoobi password is required; run `yoobi-cli auth login`".into(),
            })?;
            if base != saved.base_url || user != saved.username {
                return Err(Error::Auth{message:"saved password belongs to a different Yoobi URL or username; log in for that account or provide an explicit password".into()});
            }
            store.get(&saved.account())?.ok_or_else(||Error::Auth{message:"saved password is missing from the OS credential store; run `yoobi-cli auth login`".into()})?
        }
    };
    Config::new(&base, user.to_owned(), password)
}
pub fn normalize_base(base: &str) -> Result<String, Error> {
    let url = Url::parse(base).map_err(|_| Error::Usage {
        message: "YOOBI_BASE_URL must be an HTTPS origin".into(),
    })?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(Error::Usage { message: "base URL must be an HTTPS origin without credentials, path, query or fragment (HTTP allowed only on loopback)".into() });
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

//! Credentials are read only from explicit environment variables or a private file.
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
    pub fn from_env() -> Result<Self, Error> {
        let base = env::var("YOOBI_BASE_URL").unwrap_or_default();
        let user = env::var("YOOBI_USERNAME").unwrap_or_default();
        let password = match (
            env::var("YOOBI_PASSWORD").ok(),
            env::var_os("YOOBI_PASSWORD_FILE"),
        ) {
            (Some(_), Some(_)) => {
                return Err(Error::Usage {
                    message: "set only one of YOOBI_PASSWORD and YOOBI_PASSWORD_FILE".into(),
                });
            }
            (Some(value), None) => value,
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
                    .map_err(|_| Error::Auth {
                        message: "cannot read password file".into(),
                    })?
                    .trim_end_matches(['\r', '\n'])
                    .to_owned()
            }
            _ => String::new(),
        };
        Self::new(&base, user, password)
    }
    pub fn status() -> Value {
        json!({"configured":Self::from_env().is_ok(), "source":"environment", "verified":false})
    }
    pub(crate) fn password(&self) -> &str {
        &self.password
    }
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

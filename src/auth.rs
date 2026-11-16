//! Saved identity metadata and OS credential storage. Passwords never enter config files.
use crate::{
    Error,
    api::Client,
    config::{Config, normalize_base},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{IsTerminal, Read, Write},
    path::{Path, PathBuf},
};

const SERVICE: &str = "yoobi-cli/login/v1";
fn auth_error(message: &str) -> Error {
    Error::Auth {
        message: message.into(),
    }
}
fn local_error(message: &str) -> Error {
    Error::Io {
        message: message.into(),
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedLogin {
    version: u8,
    pub base_url: String,
    pub username: String,
    credential_id: String,
}
impl SavedLogin {
    pub(crate) fn account(&self) -> String {
        serde_json::to_string(&(&self.base_url, &self.username, &self.credential_id))
            .expect("strings serialize")
    }
    fn validate(&self) -> Result<(), Error> {
        if self.version != 1
            || self.username.trim().is_empty()
            || normalize_base(&self.base_url)? != self.base_url
            || uuid::Uuid::parse_str(&self.credential_id).is_err()
        {
            return Err(auth_error(
                "saved login metadata is invalid; restore or remove login.json before logging in again",
            ));
        }
        Ok(())
    }
}
pub(crate) trait Credentials {
    fn get(&self, account: &str) -> Result<Option<String>, Error>;
    fn set(&self, account: &str, password: &str) -> Result<(), Error>;
    fn delete(&self, account: &str) -> Result<(), Error>;
}
pub(crate) struct NativeCredentials;
impl Credentials for NativeCredentials {
    fn get(&self, account: &str) -> Result<Option<String>, Error> {
        let entry = keyring::Entry::new(SERVICE, account)
            .map_err(|_| auth_error("cannot open the OS credential store"))?;
        match entry.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(auth_error(
                "cannot read the OS credential store; unlock it and try again",
            )),
        }
    }
    fn set(&self, account: &str, password: &str) -> Result<(), Error> {
        keyring::Entry::new(SERVICE, account)
            .and_then(|e| e.set_password(password))
            .map_err(|_| {
                auth_error(
                    "cannot save to the OS credential store; credentials were not saved to a file",
                )
            })
    }
    fn delete(&self, account: &str) -> Result<(), Error> {
        let entry = keyring::Entry::new(SERVICE, account)
            .map_err(|_| auth_error("cannot open the OS credential store"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(auth_error(
                "cannot remove the saved password; unlock the OS credential store and retry logout",
            )),
        }
    }
}
pub(crate) fn config_dir() -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os("YOOBI_CONFIG_DIR") {
        if path.is_empty() {
            return Err(local_error("YOOBI_CONFIG_DIR must not be empty"));
        }
        return Ok(PathBuf::from(path));
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("yoobi-cli"))
        .ok_or_else(|| local_error("cannot locate the user config directory; set YOOBI_CONFIG_DIR"))
}
pub(crate) fn read_saved(dir: &Path) -> Result<Option<SavedLogin>, Error> {
    let path = dir.join("login.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(local_error("cannot inspect saved login metadata")),
    };
    if !metadata.file_type().is_file() {
        return Err(local_error(
            "saved login metadata must be a regular file, not a symlink",
        ));
    }
    let content = fs::read(path).map_err(|_| local_error("cannot read saved login metadata"))?;
    let saved: SavedLogin = serde_json::from_slice(&content).map_err(|_| {
        auth_error(
            "saved login metadata is invalid; restore or remove login.json before logging in again",
        )
    })?;
    saved.validate()?;
    Ok(Some(saved))
}
fn lock(dir: &Path) -> Result<File, Error> {
    fs::create_dir_all(dir).map_err(|_| local_error("cannot create the login config directory"))?;
    let meta = fs::symlink_metadata(dir)
        .map_err(|_| local_error("cannot inspect the login config directory"))?;
    if !meta.file_type().is_dir() {
        return Err(local_error(
            "login config directory must be a directory, not a symlink",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| local_error("cannot make the login config directory private"))?;
    }
    let path = dir.join("login.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(local_error(
            "login lock must be a regular file, not a symlink",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|_| local_error("cannot open the login lock"))?;
    file.lock()
        .map_err(|_| local_error("cannot lock saved login metadata"))?;
    Ok(file)
}
pub(crate) fn save(config: &Config, dir: &Path, store: &impl Credentials) -> Result<Value, Error> {
    let _lock = lock(dir)?;
    let previous = read_saved(dir)?;
    let saved = SavedLogin {
        version: 1,
        base_url: config.base_url.clone(),
        username: config.username.clone(),
        credential_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut staged = tempfile::NamedTempFile::new_in(dir)
        .map_err(|_| local_error("cannot prepare saved login metadata"))?;
    serde_json::to_writer_pretty(&mut staged, &saved)
        .map_err(|_| local_error("cannot encode saved login metadata"))?;
    staged
        .write_all(b"\n")
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|_| local_error("cannot write saved login metadata"))?;
    store.set(&saved.account(), config.password())?;
    if staged.persist(dir.join("login.json")).is_err() {
        if store.delete(&saved.account()).is_err() {
            return Err(local_error(
                "could not save login metadata or remove the new OS credential entry; the previous login is unchanged",
            ));
        }
        return Err(local_error(
            "could not save login metadata; the previous login is unchanged",
        ));
    }
    let cleanup_pending = previous.is_some_and(|old| store.delete(&old.account()).is_err());
    Ok(json!({"saved":true,"verified":true,"source":"keychain","cleanup_pending":cleanup_pending}))
}
pub(crate) fn logout(dir: &Path, store: &impl Credentials) -> Result<Value, Error> {
    if !dir.exists() {
        return Ok(json!({"saved":false,"removed":false}));
    }
    let _lock = lock(dir)?;
    let Some(saved) = read_saved(dir)? else {
        return Ok(json!({"saved":false,"removed":false}));
    };
    store.delete(&saved.account())?;
    fs::remove_file(dir.join("login.json"))
        .map_err(|_| local_error("password removed but login metadata remains; retry logout"))?;
    Ok(json!({"saved":false,"removed":true}))
}
fn prompt(label: &str) -> Result<String, Error> {
    eprint!("{label}: ");
    std::io::stderr()
        .flush()
        .map_err(|_| local_error("cannot write login prompt"))?;
    let mut value = String::new();
    std::io::stdin()
        .read_line(&mut value)
        .map_err(|_| local_error("cannot read login input"))?;
    Ok(value.trim().to_owned())
}
fn tty_required() -> Error {
    Error::TtyRequired{message:"login needs a terminal to prompt; supply URL, username and YOOBI_PASSWORD/YOOBI_PASSWORD_FILE, or use --password-stdin".into()}
}
pub fn login(
    base_url: Option<String>,
    username: Option<String>,
    password_stdin: bool,
) -> Result<Value, Error> {
    let dir = config_dir()?;
    let saved = read_saved(&dir)?;
    let interactive = std::io::stdin().is_terminal();
    let base = base_url
        .or_else(|| std::env::var("YOOBI_BASE_URL").ok())
        .or_else(|| saved.as_ref().map(|s| s.base_url.clone()));
    let user = username
        .or_else(|| std::env::var("YOOBI_USERNAME").ok())
        .or_else(|| saved.as_ref().map(|s| s.username.clone()));
    if !interactive && (base.is_none() || user.is_none()) {
        return Err(tty_required());
    }
    let base = match base {
        Some(b) => b,
        None => prompt("Yoobi URL (https://your-company.yoobi.nl)")?,
    };
    let user = match user {
        Some(u) => u,
        None => prompt("Yoobi username")?,
    };
    // Validate destination before prompting for or consuming a password.
    normalize_base(&base)?;
    if user.trim().is_empty() {
        return Err(auth_error("Yoobi username is required"));
    }
    let password = if password_stdin {
        if std::env::var_os("YOOBI_PASSWORD").is_some()
            || std::env::var_os("YOOBI_PASSWORD_FILE").is_some()
        {
            return Err(Error::Usage {
                message: "--password-stdin conflicts with YOOBI_PASSWORD and YOOBI_PASSWORD_FILE"
                    .into(),
            });
        }
        let mut password = String::new();
        std::io::stdin()
            .take(16385)
            .read_to_string(&mut password)
            .map_err(|_| local_error("cannot read password from stdin"))?;
        if password.len() > 16384 {
            return Err(Error::InvalidInput {
                message: "password exceeds 16 KiB".into(),
            });
        }
        password.trim_end_matches(['\r', '\n']).to_owned()
    } else if let Some(password) = crate::config::environment_password()? {
        password
    } else {
        if !interactive {
            return Err(tty_required());
        }
        eprint!("Yoobi password: ");
        std::io::stderr()
            .flush()
            .map_err(|_| local_error("cannot write login prompt"))?;
        rpassword::read_password().map_err(|_| local_error("cannot read password from terminal"))?
    };
    let config = Config::new(&base, user, password)?;
    // Login verification uses only security/doLogin + timesheet/index; no declarations are saved.
    Client::new(&config)?;
    save(&config, &dir, &NativeCredentials)
}
pub fn forget() -> Result<Value, Error> {
    logout(&config_dir()?, &NativeCredentials)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };
    #[derive(Default)]
    struct Memory {
        values: RefCell<BTreeMap<String, String>>,
        reads: Cell<usize>,
        fail_set: Cell<bool>,
        fail_delete: Cell<bool>,
        block_metadata: Option<PathBuf>,
    }
    impl Credentials for Memory {
        fn get(&self, a: &str) -> Result<Option<String>, Error> {
            self.reads.set(self.reads.get() + 1);
            Ok(self.values.borrow().get(a).cloned())
        }
        fn set(&self, a: &str, p: &str) -> Result<(), Error> {
            if self.fail_set.get() {
                return Err(auth_error("store locked"));
            }
            self.values.borrow_mut().insert(a.into(), p.into());
            if let Some(dir) = &self.block_metadata {
                fs::create_dir(dir.join("login.json")).unwrap();
            }
            Ok(())
        }
        fn delete(&self, a: &str) -> Result<(), Error> {
            if self.fail_delete.get() {
                return Err(auth_error("store locked"));
            }
            self.values.borrow_mut().remove(a);
            Ok(())
        }
    }
    fn config(secret: &str) -> Config {
        Config::new("https://tenant.example", "user".into(), secret.into()).unwrap()
    }
    #[test]
    fn persisted_login_reloads_without_plaintext_password_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        let result = save(&config("private-password"), dir.path(), &store).unwrap();
        assert_eq!(result["saved"], true);
        let raw = fs::read_to_string(dir.path().join("login.json")).unwrap();
        assert!(!raw.contains("private-password"));
        assert!(!raw.contains("password"));
        assert!(!result.to_string().contains("private-password"));
        let saved = read_saved(dir.path()).unwrap().unwrap();
        let loaded = crate::config::resolve(None, None, None, Some(&saved), &store).unwrap();
        assert_eq!(loaded.password(), "private-password");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.path().join("login.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
    }
    #[test]
    fn origin_and_user_overrides_never_reuse_another_identitys_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        save(&config("private-password"), dir.path(), &store).unwrap();
        let saved = read_saved(dir.path()).unwrap().unwrap();
        for (url, user) in [
            (Some("https://other.example"), None),
            (None, Some("other-user")),
        ] {
            assert!(crate::config::resolve(url, user, None, Some(&saved), &store).is_err());
        }
        assert_eq!(store.reads.get(), 0);
        let config = crate::config::resolve(
            Some("https://other.example"),
            None,
            Some("explicit-password".into()),
            Some(&saved),
            &store,
        )
        .unwrap();
        assert_eq!(config.password(), "explicit-password");
        assert_eq!(store.reads.get(), 0);
    }
    #[test]
    fn modified_metadata_cannot_send_an_existing_secret_to_another_origin() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        save(&config("private-password"), dir.path(), &store).unwrap();
        let mut saved = read_saved(dir.path()).unwrap().unwrap();
        saved.base_url = "https://other.example".into();
        assert!(crate::config::resolve(None, None, None, Some(&saved), &store).is_err());
    }
    #[test]
    fn replacing_login_cleans_previous_secret_and_failed_save_preserves_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        save(&config("old"), dir.path(), &store).unwrap();
        let old = read_saved(dir.path()).unwrap().unwrap();
        store.fail_set.set(true);
        assert!(save(&config("new"), dir.path(), &store).is_err());
        assert_eq!(
            read_saved(dir.path()).unwrap().unwrap().credential_id,
            old.credential_id
        );
        assert_eq!(store.get(&old.account()).unwrap().as_deref(), Some("old"));
        store.fail_set.set(false);
        save(&config("new"), dir.path(), &store).unwrap();
        assert!(store.get(&old.account()).unwrap().is_none());
        assert_eq!(store.values.borrow().len(), 1);
    }
    #[test]
    fn metadata_failure_removes_the_staged_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory {
            block_metadata: Some(dir.path().to_path_buf()),
            ..Memory::default()
        };
        assert!(save(&config("new"), dir.path(), &store).is_err());
        assert!(store.values.borrow().is_empty());
    }
    #[test]
    fn cleanup_failure_is_reported_after_successful_login_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        save(&config("old"), dir.path(), &store).unwrap();
        store.fail_delete.set(true);
        let result = save(&config("new"), dir.path(), &store).unwrap();
        assert_eq!(result["saved"], true);
        assert_eq!(result["cleanup_pending"], true);
    }
    #[test]
    fn logout_removes_both_stores_is_idempotent_and_retains_metadata_on_keychain_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Memory::default();
        save(&config("secret"), dir.path(), &store).unwrap();
        store.fail_delete.set(true);
        assert!(logout(dir.path(), &store).is_err());
        assert!(read_saved(dir.path()).unwrap().is_some());
        store.fail_delete.set(false);
        assert_eq!(logout(dir.path(), &store).unwrap()["removed"], true);
        assert!(!dir.path().join("login.json").exists());
        assert!(store.values.borrow().is_empty());
        assert_eq!(logout(dir.path(), &store).unwrap()["removed"], false);
    }
    #[test]
    fn unknown_config_fields_and_symlinks_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("login.json"),r#"{"version":1,"base_url":"https://example.com","username":"user","credential_id":"00000000-0000-4000-8000-000000000000","password":"do-not-accept-plaintext"}"#).unwrap();
        assert!(read_saved(dir.path()).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = tempfile::tempdir().unwrap();
            let link = outside.path().join("linked");
            symlink(dir.path(), &link).unwrap();
            assert!(save(&config("secret"), &link, &Memory::default()).is_err());
        }
    }
}

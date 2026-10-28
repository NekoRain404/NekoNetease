//! The OS credential store's bound session format.

use super::StoredSession;

const KEYRING_SERVICE: &str = "NekoNetease";
const KEYRING_USER: &str = "netease-session";
pub(super) const SESSION_VERSION: u8 = 1;

pub(super) fn valid_device_id(value: &str) -> bool {
    value.len() == 52 && value.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
}

pub(super) fn decode_session(value: &str) -> Option<StoredSession> {
    let session = serde_json::from_str::<StoredSession>(value).ok()?;
    (session.version == SESSION_VERSION
        && valid_device_id(&session.device_id)
        && !session.cookie.is_empty()
        && !session.cookie.contains(['\r', '\n']))
    .then_some(session)
}

pub(super) fn read_credential() -> Result<Option<String>, ()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|_| ())?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(()),
    }
}

pub(super) fn write_credential(value: &str) -> Result<(), ()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|_| ())?;
    entry.set_password(value).map_err(|_| ())
}

pub(super) fn delete_credential() -> Result<(), ()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|_| ())?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(()),
    }
}

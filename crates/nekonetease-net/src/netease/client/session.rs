use std::collections::BTreeMap;

use nekonetease_core::entities::cloud::Account;
use serde_json::{Value, json};

use crate::netease::endpoints::{self as ep, Protocol};
use crate::netease::{Error, Result, crypto};

use super::decode::decode;
use super::{Client, QrStatus};

impl Client {
    pub async fn initialize_guest(&self) -> Result<()> {
        self.request(
            ep::REGISTER_ANONYMOUS,
            json!({"username":crypto::anonymous_username(&self.device_id)}),
        )
        .await?;
        Ok(())
    }

    /// Submit one actual listen. Writes are deliberately not retried: a lost acknowledgement
    /// must not turn into a duplicate listen. Keep both logs in the same account generation.
    pub async fn report_listen(&self, song_id: i64, seconds: u64) -> Result<()> {
        if song_id <= 0 || seconds == 0 {
            return Err(Error::InvalidResponse);
        }
        let generation = self.generation();
        if self.cookies.read().await.get("MUSIC_U").is_none_or(String::is_empty) {
            return Err(Error::Authentication);
        }
        for action in ["startplay", "play"] {
            if generation != self.generation() {
                return Err(Error::Cancelled);
            }
            self.request_in_generation(
                ep::WEBLOG,
                listen_log(action, song_id, seconds)?,
                generation,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn account(&self) -> Result<Account> {
        let body = self.request(ep::ACCOUNT, json!({})).await?;
        decode(body.get("profile").cloned().filter(|v| !v.is_null()).ok_or(Error::Authentication)?)
    }

    pub async fn login_phone(
        &self,
        phone: &str,
        country_code: &str,
        password: &str,
    ) -> Result<Account> {
        self.request(ep::PHONE_LOGIN,json!({"type":"1","https":"true","phone":phone,"countrycode":country_code,"password":crypto::md5_hex(password.as_bytes()),"remember":"true"})).await?;
        self.account().await
    }

    /// Sends the SMS login code. The route's `secrete` tag is the desktop client's login flow;
    /// omitting it lets the server apply the web flow's stricter risk checks.
    pub async fn send_login_captcha(&self, phone: &str, country_code: &str) -> Result<()> {
        self.request_at_write(
            SMS_CAPTCHA_SENT,
            Protocol::Weapi,
            json!({
                "cellphone": phone,
                "ctcode": country_code,
                "secrete": "music_middleuser_pclogin",
            }),
        )
        .await
        .map(|_| ())
        .map_err(captcha_send_error)
    }

    /// Signs in with the SMS code instead of the password. `PHONE_LOGIN` is the same route the
    /// password flow uses; the server chooses the credential from which field is present.
    pub async fn login_phone_captcha(
        &self,
        phone: &str,
        country_code: &str,
        captcha: &str,
    ) -> Result<Account> {
        self.request(
            ep::PHONE_LOGIN,
            json!({
                "type": "1",
                "https": "true",
                "phone": phone,
                "countrycode": country_code,
                "captcha": captcha,
                "remember": "true",
            }),
        )
        .await
        .map_err(captcha_login_error)?;
        self.account().await
    }

    pub async fn login_email(&self, email: &str, password: &str) -> Result<Account> {
        self.request(ep::EMAIL_LOGIN,json!({"type":"0","https":"true","username":email,"password":crypto::md5_hex(password.as_bytes()),"rememberLogin":"true"})).await?;
        self.account().await
    }

    pub async fn qr_key(&self) -> Result<String> {
        let body = self.request(ep::QR_KEY, json!({"type":3})).await?;
        body["unikey"].as_str().map(str::to_owned).ok_or(Error::InvalidResponse)
    }

    pub async fn qr_check(&self, key: &str) -> Result<QrStatus> {
        let body = self.request(ep::QR_CHECK, json!({"key":key,"type":3})).await?;
        match body["code"].as_i64() {
            Some(800) => Ok(QrStatus::Expired),
            Some(801) => Ok(QrStatus::Waiting),
            Some(802) => Ok(QrStatus::Confirming),
            Some(803) => Ok(QrStatus::Authorized),
            _ => Err(Error::InvalidResponse),
        }
    }

    pub async fn refresh(&self) -> Result<()> {
        self.request(ep::REFRESH, json!({})).await?;
        Ok(())
    }

    pub async fn logout(&self) -> Result<()> {
        let result = self.request(ep::LOGOUT, json!({})).await;
        self.clear_session().await;
        result.map(|_| ())
    }
}

const SMS_CAPTCHA_SENT: &str = "/api/sms/captcha/sent";

pub(super) fn captcha_send_error(error: Error) -> Error {
    match error {
        // The send route answers 503/509 for its own resend limit, which is not a login rejection.
        Error::Restricted | Error::Service(503 | 509) => Error::CaptchaSend,
        other => other,
    }
}

pub(super) fn captcha_login_error(error: Error) -> Error {
    match error {
        Error::Credentials => Error::CaptchaInvalid,
        other => other,
    }
}

pub(super) fn listen_log(action: &str, song_id: i64, seconds: u64) -> Result<Value> {
    // A cloud queue can mix search, album and playlist results; it does not retain a single
    // upstream source ID. Report the unknown source as 0 instead of inventing a playlist.
    let mut data = json!({
        "id": song_id.to_string(), "type": "song", "mainsite": "1",
        "mainsiteWeb": "1", "content": "id=0"
    });
    if action == "play" {
        data["download"] = json!(0);
        data["end"] = json!("playend");
        data["sourceId"] = json!("0");
        data["time"] = json!(seconds);
        data["wifi"] = json!(0);
        data["source"] = json!("list");
    }
    let logs = serde_json::to_string(&json!([{ "action": action, "json": data }]))
        .map_err(|_| Error::InvalidResponse)?;
    Ok(json!({"logs": logs}))
}

pub(super) fn valid_cookie_pair(name: &str, value: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !value.bytes().any(|b| b == b'\r' || b == b'\n' || b == b';')
}

pub(super) fn merge_set_cookie(cookies: &mut BTreeMap<String, String>, header: &str) {
    let Some((name, value)) = header.split(';').next().and_then(|v| v.trim().split_once('='))
    else {
        return;
    };
    if !valid_cookie_pair(name, value) {
        return;
    }
    if value.is_empty() || header.split(';').any(|v| v.trim().eq_ignore_ascii_case("max-age=0")) {
        cookies.remove(name);
    } else {
        cookies.insert(name.to_owned(), value.to_owned());
    }
}

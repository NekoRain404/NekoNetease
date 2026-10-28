use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::header::{COOKIE, SET_COOKIE};
use serde_json::{Value, json};

use crate::netease::endpoints::{self as ep, Endpoint, Protocol};
use crate::netease::{Error, Result, crypto};

use super::Client;
use super::session::merge_set_cookie;

const ANDROID_UA: &str = "NeteaseMusic/9.5.61.260802021928(9005061);Dalvik/2.1.0 (Linux; U; Android 12; HBN-AL00 Build/cd737a2.0)";
const WEAPI_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";
pub(super) const EAPI_UA: &str = "NeteaseMusic 9.0.90/5038 (iPhone; iOS 16.2; zh_CN)";
pub(super) const DESKTOP_APPVER: &str = "3.1.17.204416";
const DESKTOP_OSVER: &str = "Microsoft-Windows-10-Professional-build-19045-64bit";
const MAX_JSON: u64 = 16 * 1024 * 1024;

fn request_profile(
    protocol: Protocol,
) -> (&'static str, &'static str, &'static str, &'static str, &'static str) {
    match protocol {
        Protocol::Weapi => ("pc", DESKTOP_APPVER, DESKTOP_OSVER, "netease", WEAPI_UA),
        Protocol::Eapi => ("pc", DESKTOP_APPVER, DESKTOP_OSVER, "netease", EAPI_UA),
        Protocol::Xeapi => ("android", "9.5.61", "12", "netease", ANDROID_UA),
    }
}

impl Client {
    pub(super) async fn prepare_request(
        &self,
        endpoint: Endpoint<'_>,
        mut data: Value,
        cookies: &BTreeMap<String, String>,
    ) -> Result<reqwest::RequestBuilder> {
        let csrf = cookies.get("__csrf").cloned().unwrap_or_default();
        data["e_r"] = json!(false);
        let now = chrono_millis();
        let (cookie_header, headers, user_agent) =
            self.request_context(endpoint, cookies, &csrf, now);
        let (base, route, payload) = match endpoint.protocol {
            Protocol::Weapi => {
                data["csrf_token"] = json!(csrf);
                (
                    "https://music.163.com",
                    endpoint.path.replacen("/api/", "/weapi/", 1),
                    crypto::weapi(&data)?,
                )
            }
            Protocol::Eapi => {
                data["header"] = headers;
                (
                    if endpoint.path == ep::WEBLOG.path {
                        "https://clientlog.music.163.com"
                    } else {
                        "https://interfacepc.music.163.com"
                    },
                    endpoint.path.replacen("/api/", "/eapi/", 1),
                    crypto::eapi(endpoint.path, &data)?,
                )
            }
            Protocol::Xeapi => {
                let key = self.bootstrap_xeapi().await?;
                (
                    "https://interface3.music.163.com",
                    endpoint.path.replacen("/api/", "/xeapi/", 1),
                    crypto::xeapi(&data, &key)?,
                )
            }
        };
        let url = crate::services::net::http_url(&format!("{base}{route}"))
            .ok_or(Error::InvalidResponse)?;
        let mut request = self
            .http()?
            .post(url.clone())
            .timeout(Duration::from_secs(15))
            .header("User-Agent", user_agent)
            .header(COOKIE, cookie_header)
            .form(&payload);
        if endpoint.protocol == Protocol::Weapi {
            request = request.header("Referer", "https://music.163.com/");
        }
        if endpoint.protocol == Protocol::Xeapi {
            request = request
                .header("X-Client-Enc-State", "ENCRYPTED")
                .header("x-aeapi", "true")
                .header("x-deviceid", &self.device_id)
                .header("x-os", "android")
                .header("x-osver", "12")
                .header("x-appver", "9.5.61")
                .header("x-sdeviceid", &self.device_id)
                .header("x-buildver", (chrono_millis() / 1000).to_string());
            if let Some(cookie) = cookies.get("MUSIC_U") {
                request = request.header("x-music-u", cookie);
            }
        }
        Ok(request)
    }

    pub(super) fn request_context(
        &self,
        endpoint: Endpoint<'_>,
        cookies: &BTreeMap<String, String>,
        csrf: &str,
        now: u128,
    ) -> (String, Value, &'static str) {
        // The reporting service uses the macOS desktop profile, as in API Enhanced and
        // YesPlayMusic. The ordinary EAPI host/profile does not record listening history.
        let (os, appver, osver, channel, user_agent) = if endpoint.path == ep::WEBLOG.path {
            ("osx", "3.1.10.5100", "15.5", "netease", WEAPI_UA)
        } else {
            request_profile(endpoint.protocol)
        };
        let build_version = (now / 1000).to_string();
        let mut request_cookies = cookies.clone();
        for (name, value) in [
            ("os", os),
            ("appver", appver),
            ("osver", osver),
            ("channel", channel),
            ("versioncode", "140"),
            ("mobilename", ""),
            ("resolution", "1920x1080"),
            ("__remember_me", "true"),
            ("ntes_kaola_ad", "1"),
            ("WEVNSM", "1.0.0"),
        ] {
            request_cookies.insert(name.to_owned(), value.to_owned());
        }
        request_cookies.insert("deviceId".to_owned(), self.device_id.clone());
        request_cookies.insert("buildver".to_owned(), build_version.clone());
        request_cookies.insert("__csrf".to_owned(), csrf.to_owned());
        let nuid = crypto::hex(&rand::random::<[u8; 32]>(), false);
        request_cookies.insert("_ntes_nuid".to_owned(), nuid.clone());
        request_cookies.insert("_ntes_nnid".to_owned(), format!("{nuid},{now}"));
        request_cookies.insert("WNMCID".to_owned(), self.wnmcid.clone());
        if !request_cookies.contains_key("MUSIC_U") {
            request_cookies.entry("MUSIC_A".to_owned()).or_default();
        }
        let cookie_header = request_cookies
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("; ");

        let mut headers = json!({
            "os":os, "appver":appver, "osver":osver, "deviceId":self.device_id,
            "channel":channel, "__csrf":csrf, "versioncode":"140", "mobilename":"",
            "buildver":build_version, "resolution":"1920x1080",
            "requestId":format!("{}_{}",now,format!("{:04}",rand::random::<u16>()%1000))
        });
        if let Some(value) = request_cookies.get("MUSIC_U").filter(|value| !value.is_empty()) {
            headers["MUSIC_U"] = json!(value);
        }
        if let Some(value) = request_cookies.get("MUSIC_A").filter(|value| !value.is_empty()) {
            headers["MUSIC_A"] = json!(value);
        }
        if endpoint.protocol == Protocol::Eapi
            && let Some(value) = request_cookies.get("NMTID").filter(|value| !value.is_empty())
        {
            headers["NMTID"] = json!(value);
        }
        (cookie_header, headers, user_agent)
    }

    pub(super) async fn request(&self, endpoint: Endpoint<'_>, data: Value) -> Result<Value> {
        self.request_in_generation(endpoint, data, self.generation()).await
    }

    pub(super) async fn request_in_generation(
        &self,
        endpoint: Endpoint<'_>,
        data: Value,
        generation: u64,
    ) -> Result<Value> {
        let cookies = self.cookies.read().await.clone();
        let prepared = self.prepare_request(endpoint, data, &cookies).await?;
        let attempts = if endpoint.read_only { 2 } else { 1 };
        for attempt in 0..attempts {
            if generation != self.generation() {
                return Err(Error::Cancelled);
            }
            let request = prepared.try_clone().ok_or(Error::InvalidResponse)?;
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    if attempt + 1 < attempts {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        continue;
                    }
                    return Err(if error.is_timeout() { Error::Timeout } else { Error::Transport });
                }
            };
            let status = response.status();
            if status.is_server_error() && attempt + 1 < attempts {
                tokio::time::sleep(Duration::from_millis(300)).await;
                continue;
            }
            let set_cookie = response
                .headers()
                .get_all(SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let bytes = crate::services::net::read_capped(response, "NetEase response", MAX_JSON)
                .await
                .map_err(|_| Error::Transport)?;
            if generation != self.generation() {
                return Err(Error::Cancelled);
            }
            let body = match serde_json::from_slice::<Value>(&bytes) {
                Ok(body) => body,
                Err(_) => serde_json::from_slice(&crypto::decrypt_response(&bytes)?)
                    .map_err(|_| Error::InvalidResponse)?,
            };
            if !status.is_success() {
                return Err(classify(body["code"].as_i64().unwrap_or(i64::from(status.as_u16()))));
            }
            let code = body["code"].as_i64().ok_or(Error::InvalidResponse)?;
            if code != 200 && !(800..=803).contains(&code) {
                return Err(classify(code));
            }
            let mut stored = self.cookies.write().await;
            if generation != self.generation() {
                return Err(Error::Cancelled);
            }
            for header in set_cookie {
                merge_set_cookie(&mut stored, &header);
            }
            return Ok(body);
        }
        Err(Error::Transport)
    }

    async fn bootstrap_xeapi(&self) -> Result<crypto::XeapiKey> {
        let mut key = self.xeapi_key.lock().await;
        if let Some(key) = key.as_ref() {
            return Ok(key.clone());
        }
        let nonce = rand::random::<[u8; 16]>()
            .iter()
            .map(|b| char::from(b'0' + b % 10))
            .collect::<String>();
        let timestamp = chrono_millis().to_string();
        let signature = crypto::xeapi_signature(&timestamp, &nonce)?;
        let url = crate::services::net::http_url("https://interface.music.163.com/api/bsr/sk/get")
            .ok_or(Error::InvalidResponse)?;
        let response = self
            .http()?
            .post(url)
            .header("User-Agent", ANDROID_UA)
            .timeout(Duration::from_secs(15))
            .form(&[
                ("appVersion", "9.5.61"),
                ("currentKeyVersion", ""),
                ("deviceId", self.device_id.as_str()),
                ("nonce", &nonce),
                ("os", "android"),
                ("requestType", "active"),
                ("signature", &signature),
                ("t1", ""),
                ("t2", ""),
                ("timestamp", &timestamp),
                ("uid", ""),
            ])
            .send()
            .await
            .map_err(|e| if e.is_timeout() { Error::Timeout } else { Error::Transport })?;
        let bytes = crate::services::net::read_capped(response, "NetEase key", 64 * 1024)
            .await
            .map_err(|_| Error::Transport)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)?;
        if value["code"] != 200 {
            return Err(Error::InvalidResponse);
        }
        let response_time = value["data"]["timestamp"]
            .as_str()
            .map_or_else(|| value["data"]["timestamp"].to_string(), str::to_owned);
        crypto::verify_xeapi_signature(
            &response_time,
            &nonce,
            value["data"]["signature"].as_str().ok_or(Error::InvalidResponse)?,
        )?;
        let state = crypto::decrypt_public_key(
            value["data"]["encryptedData"].as_str().ok_or(Error::InvalidResponse)?,
        )?;
        *key = Some(state.clone());
        Ok(state)
    }

    pub(super) async fn request_at(
        &self,
        path: &str,
        protocol: Protocol,
        data: Value,
    ) -> Result<Value> {
        self.request(Endpoint::read(path, protocol), data).await
    }

    pub(super) async fn request_at_write(
        &self,
        path: &str,
        protocol: Protocol,
        data: Value,
    ) -> Result<Value> {
        self.request(Endpoint::write(path, protocol), data).await
    }

    pub(super) async fn request_path(&self, path: &str, data: Value) -> Result<Value> {
        // Dynamic entity paths remain WEAPI; no leaked allocations or arbitrary URLs.
        self.request_at(path, Protocol::Weapi, data).await
    }
}

pub(super) fn classify(code: i64) -> Error {
    match code {
        301 | 401 => Error::Authentication,
        403 => Error::Permission,
        405 | 406 | 509 => Error::Restricted,
        460 | 462 | -460 | -462 => Error::RiskControl,
        502 => Error::Credentials,
        _ => Error::Service(code),
    }
}

pub(super) fn chrono_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

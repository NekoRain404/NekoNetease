//! Wire-compatible cryptography, using `RustCrypto` implementations rather than a JS runtime.

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Aes256};
use aes_gcm::aead::Aead;
use aes_gcm::{Aes128Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use serde_json::{Value, json};
use sha2::Sha256;

use super::{Error, Result};

const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";
const XEAPI_KEY: [u8; 32] = [
    0xab, 0x1d, 0x5a, 0x43, 0x0f, 0x6b, 0xb0, 0x4a, 0x3f, 0x01, 0xe8, 0x1d, 0xdd, 0x72, 0xbd, 0x91,
    0x6d, 0x5c, 0xe5, 0x91, 0x24, 0x8a, 0xc1, 0x28, 0x71, 0x48, 0x06, 0xd7, 0xf8, 0xfb, 0x1b, 0x84,
];
const SIGN_KEY: &[u8] =
    b"mUHCwVNWJbunMqAHf5MImuirT6plvs6VSFW62MGHstFQxhBGdEoIhLItH3djc4+FB/OKty3+lL2rGeoFBpVe5g==";
const RSA_MODULUS: &str = "e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b725152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0312ecbda92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce10b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0a22b8e7";

fn padded(input: &[u8]) -> Vec<u8> {
    let mut bytes = input.to_vec();
    let padding = 16 - bytes.len() % 16;
    bytes.resize(bytes.len() + padding, u8::try_from(padding).unwrap_or(16));
    bytes
}

fn encrypt128(input: &[u8], key: &[u8; 16], iv: Option<[u8; 16]>) -> Vec<u8> {
    let cipher = Aes128::new(key.into());
    let mut output = padded(input);
    let mut previous = iv;
    for block in output.as_chunks_mut::<16>().0 {
        if let Some(previous) = previous {
            for (byte, prior) in block.iter_mut().zip(previous) {
                *byte ^= prior;
            }
        }
        cipher.encrypt_block(block.into());
        if previous.is_some() {
            previous = Some(*block);
        }
    }
    output
}

fn encrypt256(input: &[u8], key: &[u8; 32]) -> Vec<u8> {
    let cipher = Aes256::new(key.into());
    let mut output = padded(input);
    for block in output.as_chunks_mut::<16>().0 {
        cipher.encrypt_block(block.into());
    }
    output
}

fn decrypt(input: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    if input.is_empty() || !input.len().is_multiple_of(16) {
        return Err(Error::InvalidResponse);
    }
    let mut bytes = input.to_vec();
    match key.len() {
        16 => {
            let cipher = Aes128::new_from_slice(key).map_err(|_| Error::InvalidResponse)?;
            for block in bytes.as_chunks_mut::<16>().0 {
                cipher.decrypt_block(block.into());
            }
        }
        32 => {
            let cipher = Aes256::new_from_slice(key).map_err(|_| Error::InvalidResponse)?;
            for block in bytes.as_chunks_mut::<16>().0 {
                cipher.decrypt_block(block.into());
            }
        }
        _ => return Err(Error::InvalidResponse),
    }
    let padding = usize::from(*bytes.last().ok_or(Error::InvalidResponse)?);
    if !(1..=16).contains(&padding)
        || bytes[bytes.len() - padding..].iter().any(|&b| usize::from(b) != padding)
    {
        return Err(Error::InvalidResponse);
    }
    bytes.truncate(bytes.len() - padding);
    Ok(bytes)
}

pub fn hex(bytes: &[u8], uppercase: bool) -> String {
    let digits = if uppercase { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
    bytes
        .iter()
        .flat_map(|byte| {
            [char::from(digits[usize::from(byte >> 4)]), char::from(digits[usize::from(byte & 15)])]
        })
        .collect()
}

pub fn md5_hex(input: &[u8]) -> String {
    hex(&Md5::digest(input), false)
}

/// Build the username accepted by `NetEase`'s anonymous-device registration endpoint.
///
/// This is the same `cloudmusic_dll_encode_id` transform used by the desktop client, wrapped in
/// base64 as expected by `/api/register/anonimous`.
pub fn anonymous_username(device_id: &str) -> String {
    const XOR_KEY: &[u8] = b"3go8&$8*3*3h0k(2)2";
    let xored = device_id
        .bytes()
        .enumerate()
        .map(|(index, byte)| byte ^ XOR_KEY[index % XOR_KEY.len()])
        .collect::<Vec<_>>();
    let digest = STANDARD.encode(Md5::digest(xored));
    STANDARD.encode(format!("{device_id} {digest}"))
}

pub fn weapi(data: &Value) -> Result<Vec<(String, String)>> {
    const BASE62: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let key = rand::random::<[u8; 16]>().map(|b| BASE62[usize::from(b) % BASE62.len()]);
    weapi_with_key(data, key)
}

fn weapi_with_key(data: &Value, key: [u8; 16]) -> Result<Vec<(String, String)>> {
    let text = serde_json::to_vec(data).map_err(|_| Error::InvalidResponse)?;
    let first = STANDARD.encode(encrypt128(&text, b"0CoJUm6Qyw8W8jud", Some(*b"0102030405060708")));
    let params = STANDARD.encode(encrypt128(first.as_bytes(), &key, Some(*b"0102030405060708")));
    let reversed: Vec<_> = key.into_iter().rev().collect();
    let modulus = BigUint::parse_bytes(RSA_MODULUS.as_bytes(), 16).ok_or(Error::InvalidResponse)?;
    let encrypted = BigUint::from_bytes_be(&reversed).modpow(&BigUint::from(65537u32), &modulus);
    Ok(vec![("params".into(), params), ("encSecKey".into(), format!("{encrypted:0256x}"))])
}

pub fn eapi(path: &str, data: &Value) -> Result<Vec<(String, String)>> {
    let text = serde_json::to_string(data).map_err(|_| Error::InvalidResponse)?;
    let digest = md5_hex(format!("nobody{path}use{text}md5forencrypt").as_bytes());
    let plain = format!("{path}-36cd479b6b5-{text}-36cd479b6b5-{digest}");
    let bytes = encrypt128(plain.as_bytes(), EAPI_KEY, None);
    let hex = hex(&bytes, true);
    Ok(vec![("params".into(), hex)])
}

#[cfg(test)]
pub(super) fn decrypt_eapi_for_test(input: &[u8]) -> Result<Vec<u8>> {
    decrypt(input, EAPI_KEY)
}

#[cfg(test)]
mod anonymous_tests {
    use super::anonymous_username;

    #[test]
    fn anonymous_registration_username_matches_the_upstream_vector() {
        assert_eq!(
            anonymous_username("0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123"),
            "MDEyMzQ1Njc4OUFCQ0RFRjAxMjM0NTY3ODlBQkNERUYwMTIzNDU2Nzg5QUJDREVGMDEyMyBYa2pIc2o5dnlXcTVRNDdCdXYyVWNnPT0=",
        );
    }
}

pub fn xeapi_signature(timestamp: &str, nonce: &str) -> Result<String> {
    Ok(STANDARD.encode(hmac256(SIGN_KEY, format!("{timestamp}{nonce}").as_bytes())?))
}

fn hmac256(key: &[u8], data: &[u8]) -> Result<[u8; 32]> {
    use hmac::Mac;
    let mut mac = <hmac::Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key)
        .map_err(|_| Error::InvalidResponse)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().into())
}

pub fn verify_xeapi_signature(timestamp: &str, nonce: &str, signature: &str) -> Result<()> {
    use hmac::Mac;
    let mut mac = <hmac::Hmac<Sha256> as hmac::KeyInit>::new_from_slice(SIGN_KEY)
        .map_err(|_| Error::InvalidResponse)?;
    mac.update(format!("{timestamp}{nonce}").as_bytes());
    let bytes = STANDARD.decode(signature).map_err(|_| Error::InvalidResponse)?;
    mac.verify_slice(&bytes).map_err(|_| Error::InvalidResponse)
}

/// Public key metadata obtained from the signed `/api/bsr/sk/get` bootstrap.
#[derive(Clone)]
pub struct XeapiKey {
    pub public_key: [u8; 32],
    pub version: String,
    pub sk: String,
}

pub fn decrypt_public_key(encrypted: &str) -> Result<XeapiKey> {
    let bytes = STANDARD.decode(encrypted).map_err(|_| Error::InvalidResponse)?;
    let value: Value = serde_json::from_slice(&decrypt(&bytes, &XEAPI_KEY)?)
        .map_err(|_| Error::InvalidResponse)?;
    let public_key = STANDARD
        .decode(value["publicKey"].as_str().ok_or(Error::InvalidResponse)?)
        .map_err(|_| Error::InvalidResponse)?;
    Ok(XeapiKey {
        public_key: public_key.try_into().map_err(|_| Error::InvalidResponse)?,
        version: value["version"]
            .as_str()
            .map_or_else(|| value["version"].to_string(), str::to_owned),
        sk: value["sk"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Error::InvalidResponse)?
            .to_owned(),
    })
}

pub fn xeapi(data: &Value, peer: &XeapiKey) -> Result<Vec<(String, String)>> {
    let pairs: Vec<(String, String)> = data
        .as_object()
        .ok_or(Error::InvalidResponse)?
        .iter()
        .filter(|(name, _)| name.as_str() != "e_r")
        .map(|(name, value)| {
            (name.clone(), value.as_str().map_or_else(|| value.to_string(), str::to_owned))
        })
        .collect();
    let content = serde_urlencoded_string(&pairs)?;
    let plain = serde_json::to_vec(&json!({"content":content,"queryString":"e_r=true"}))
        .map_err(|_| Error::InvalidResponse)?;
    let dynamic = rand::random::<[u8; 16]>();
    let first = encrypt256(&plain, &XEAPI_KEY);
    let random = rand::random::<[u8; 16]>();
    let mut transformed: Vec<u8> =
        first.iter().enumerate().map(|(i, &b)| b ^ random[i & 15]).collect();
    let rotation = usize::from(random[0] & 15) % transformed.len();
    transformed.rotate_left(rotation);
    let mut middle = random.to_vec();
    middle.extend_from_slice(&transformed);
    let c = encrypt128(&middle, &dynamic, None);
    let secret = rand::random::<[u8; 32]>();
    let public = x25519_dalek::x25519(secret, x25519_dalek::X25519_BASEPOINT_BYTES);
    let shared = x25519_dalek::x25519(secret, peer.public_key);
    if shared == [0u8; 32] {
        return Err(Error::InvalidResponse);
    }
    let prk = hmac256(&[0; 32], &shared)?;
    let mut info = public.to_vec();
    info.push(1);
    let derived = hmac256(&prk, &info)?;
    let nonce = rand::random::<[u8; 12]>();
    let cipher = Aes128Gcm::new_from_slice(&derived[..16]).map_err(|_| Error::InvalidResponse)?;
    let plaintext = format!("{}|android|{}", STANDARD.encode(dynamic), peer.sk);
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| Error::InvalidResponse)?;
    let mut s = public.to_vec();
    s.extend_from_slice(&nonce);
    s.extend_from_slice(&sealed);
    let r = encrypt256(format!("{}|", peer.version).as_bytes(), &XEAPI_KEY);
    Ok(vec![
        ("C".into(), URL_SAFE_NO_PAD.encode(c)),
        ("S".into(), STANDARD.encode(s)),
        ("R".into(), STANDARD.encode(r)),
    ])
}

pub fn serde_urlencoded_string(pairs: &[(String, String)]) -> Result<String> {
    let mut url =
        reqwest::Url::parse("https://music.163.com/").map_err(|_| Error::InvalidResponse)?;
    url.query_pairs_mut().extend_pairs(pairs);
    Ok(url.query().unwrap_or_default().to_owned())
}

pub fn decrypt_response(body: &[u8]) -> Result<Vec<u8>> {
    let bytes = decrypt(body, EAPI_KEY)?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        use std::io::Read;
        let mut output = Vec::new();
        flate2::read::GzDecoder::new(bytes.as_slice())
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut output)
            .map_err(|_| Error::InvalidResponse)?;
        if output.len() > 16 * 1024 * 1024 {
            return Err(Error::InvalidResponse);
        }
        Ok(output)
    } else {
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_password_vector() {
        assert_eq!(md5_hex(b"password"), "5f4dcc3b5aa765d61d8327deb882cf99");
    }

    #[test]
    fn invalid_ciphertext_and_padding_are_rejected() {
        assert!(decrypt(&[1; 15], EAPI_KEY).is_err());
        assert!(decrypt(&[0; 16], EAPI_KEY).is_err());
    }

    #[test]
    fn eapi_payload_round_trips_wire_envelope() -> Result<()> {
        let data = json!({"id":123});
        let payload = eapi("/api/song/lyric", &data)?;
        let hex = &payload[0].1;
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| Error::InvalidResponse))
            .collect::<Result<Vec<_>>>()?;
        let plain =
            String::from_utf8(decrypt(&bytes, EAPI_KEY)?).map_err(|_| Error::InvalidResponse)?;
        assert_eq!(
            plain,
            "/api/song/lyric-36cd479b6b5-{\"id\":123}-36cd479b6b5-c27e2856ddcbfcee7c6e4908ac1ed9f9"
        );
        Ok(())
    }

    #[test]
    fn weapi_has_fixed_width_rsa_and_deterministic_ciphertext() -> Result<()> {
        let body = weapi_with_key(&json!({"id":123}), *b"abcdefghijklmnop")?;
        assert_eq!(body[1].1.len(), 256);
        assert_eq!(body, weapi_with_key(&json!({"id":123}), *b"abcdefghijklmnop")?);
        Ok(())
    }

    #[test]
    fn hmac_rfc4231_vector() -> Result<()> {
        let digest = hmac256(&[0x0b; 20], b"Hi There")?;
        assert_eq!(
            hex(&digest, false),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        Ok(())
    }
}

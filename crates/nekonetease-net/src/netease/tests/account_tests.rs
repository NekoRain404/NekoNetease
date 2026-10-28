use super::account::{DEVICE_KICKOFF, DEVICE_LIST, decode_beans, decode_check_in, decode_devices};
use super::request::classify;
use super::session::{captcha_login_error, captcha_send_error};
use crate::netease::Error;
use serde_json::json;

#[test]
fn user_info_yields_the_balance_and_the_mobile_claim() -> Result<(), Box<dyn std::error::Error>> {
    let beans = decode_beans(json!({
        "code": 200,
        "level": 8,
        "userPoint": {"balance": 321, "blockBalance": 9},
        "mobileSign": true,
        "pcSign": false
    }))?;

    assert_eq!(beans.balance, 321);
    assert!(beans.signed_today);
    Ok(())
}

#[test]
fn either_client_flag_means_the_day_is_claimed() -> Result<(), Box<dyn std::error::Error>> {
    let desktop = decode_beans(json!({
        "userPoint": {"balance": 0},
        "mobileSign": false,
        "pcSign": true
    }))?;
    assert!(desktop.signed_today);

    let neither = decode_beans(json!({
        "userPoint": {"balance": 0},
        "mobileSign": false,
        "pcSign": false
    }))?;
    assert!(!neither.signed_today);
    Ok(())
}

#[test]
fn an_account_without_a_point_row_reports_no_balance() -> Result<(), Box<dyn std::error::Error>> {
    let beans = decode_beans(json!({"code": 200, "mobileSign": false, "pcSign": false}))?;
    assert_eq!(beans.balance, 0);
    assert!(!beans.signed_today);
    Ok(())
}

#[test]
fn a_negative_balance_cannot_underflow_the_unsigned_field() -> Result<(), Box<dyn std::error::Error>>
{
    let beans = decode_beans(json!({"userPoint": {"balance": -5}}))?;
    assert_eq!(beans.balance, 0);
    Ok(())
}

#[test]
fn check_in_reads_the_sign_flag_both_ways() -> Result<(), Box<dyn std::error::Error>> {
    assert!(decode_check_in(json!({"code": 200, "data": {"sign": true}}))?);
    assert!(!decode_check_in(json!({"code": 200, "data": {"sign": false}}))?);
    Ok(())
}

#[test]
fn check_in_without_a_data_row_is_an_invalid_response() {
    assert!(decode_check_in(json!({"code": 200})).is_err());
}

#[test]
fn the_routes_are_the_verified_ones_and_the_claim_is_a_write() {
    let beans = super::ep::CLOUD_BEANS;
    assert_eq!(beans.path, "/api/v1/user/info");
    assert_eq!(beans.protocol, super::ep::Protocol::Weapi);
    assert!(beans.read_only);

    let check_in = super::ep::CLOUD_CHECK_IN;
    assert_eq!(check_in.path, "/api/pointmall/user/sign");
    assert_eq!(check_in.protocol, super::ep::Protocol::Weapi);
    assert!(!check_in.read_only);
}

#[test]
fn risk_control_codes_are_a_distinct_error() {
    for code in [-460, 460, -462, 462] {
        assert!(matches!(classify(code), Error::RiskControl), "code {code}");
    }
    assert!(matches!(classify(509), Error::Restricted));
}

#[test]
fn a_captcha_send_limit_is_not_reported_as_a_login_rejection() {
    assert!(matches!(captcha_send_error(Error::Restricted), Error::CaptchaSend));
    assert!(matches!(captcha_send_error(Error::Service(503)), Error::CaptchaSend));
    assert!(matches!(captcha_send_error(Error::RiskControl), Error::RiskControl));
}

#[test]
fn a_rejected_captcha_is_not_reported_as_a_wrong_password() {
    assert!(matches!(captcha_login_error(Error::Credentials), Error::CaptchaInvalid));
    assert!(matches!(captcha_login_error(Error::RiskControl), Error::RiskControl));
}

#[test]
fn the_device_routes_are_the_verified_ones() {
    assert_eq!(DEVICE_LIST, "/api/middle/user/device/list");
    assert_eq!(DEVICE_KICKOFF, "/api/middle/user/security/device/kickoff");
}

#[test]
fn device_rows_accept_string_and_numeric_keys_and_skip_unkeyed_rows() {
    let devices = decode_devices(&serde_json::json!({
        "code": 200,
        "data": {
            "userDeviceList": [
                {
                    "deviceKey": "desktop-key",
                    "deviceName": "NekoNetease",
                    "os": "linux",
                    "loginPlace": "Shanghai",
                    "region": "CN",
                    "lastActiveTime": 1_700_000_000_000_i64,
                    "ifLoginDevice": true
                },
                {
                    "deviceKey": 42,
                    "deviceName": "Phone",
                    "os": "android",
                    "region": "Beijing",
                    "lastActiveTime": "1700000000001",
                    "ifLoginDevice": false
                },
                {"deviceName": "No key"}
            ]
        }
    }));

    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].key, "desktop-key");
    assert_eq!(devices[0].place, "Shanghai");
    assert!(devices[0].current);
    assert_eq!(devices[1].key, "42");
    assert_eq!(devices[1].place, "Beijing");
    assert_eq!(devices[1].last_active_ms, 1_700_000_000_001);
    assert!(!devices[1].current);
}

#[test]
fn a_missing_device_list_is_empty_rather_than_an_error() {
    assert_eq!(decode_devices(&serde_json::json!({"code": 200, "data": null})), Vec::new());
    assert_eq!(decode_devices(&serde_json::json!({"code": 200})), Vec::new());
}

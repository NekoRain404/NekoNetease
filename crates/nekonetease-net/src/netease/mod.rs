//! Direct `NetEase` client. Reference: api-enhanced commit 2db6684453297ee86dec0492842f2320b08182d9.
//! No Node sidecar, response JSON or credentials cross the application facade.

mod client;
pub mod crypto;
pub mod endpoints;
pub use client::{Client, QrStatus};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("网络请求超时，请重试")]
    Timeout,
    #[error("无法连接网易云音乐，请检查网络")]
    Transport,
    #[error("登录已失效，请重新登录")]
    Authentication,
    #[error("该内容需要登录或会员权限")]
    Permission,
    #[error("网易云要求额外验证，请使用扫码登录")]
    Restricted,
    #[error("网易云风控要求完成安全验证，请稍后重试或使用扫码登录")]
    RiskControl,
    #[error("验证码发送失败，请稍后再试")]
    CaptchaSend,
    #[error("验证码错误或已过期")]
    CaptchaInvalid,
    #[error("账号或密码错误")]
    Credentials,
    #[error("歌曲暂时无法播放或没有版权")]
    Unavailable,
    #[error("该歌曲需要购买专辑或数字单曲后才能播放")]
    PurchaseRequired,
    #[error("网易云接口返回了无法识别的数据")]
    InvalidResponse,
    #[error("网易云设备标识无效")]
    InvalidDeviceId,
    #[error("网络请求已取消")]
    Cancelled,
    #[error("网易云接口暂时不可用（状态码 {0}）")]
    Service(i64),
}

impl From<Error> for nekonetease_core::error::AppError {
    fn from(error: Error) -> Self {
        Self::network_msg(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

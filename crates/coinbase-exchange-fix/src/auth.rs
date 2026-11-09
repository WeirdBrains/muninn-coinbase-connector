//! Logon HMAC. The prehash is the SOH-joined tuple
//! `SendingTime, MsgType, MsgSeqNum, SenderCompID, TargetCompID, Passphrase`
//! with no trailing separator. `RawData` is base64(HMAC-SHA256(base64-decode(secret), prehash)).

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::codec::{AppMessage, SOH};
use crate::error::{Error, Result};
use crate::tags::{
    CANCEL_ORDERS_ON_DISCONNECT, DEFAULT_APPL_VER_ID, DROP_COPY_FLAG, ENCRYPT_METHOD, HEART_BT_INT,
    PASSWORD, RAW_DATA, RAW_DATA_LENGTH, RESET_SEQ_NUM_FLAG, USERNAME,
};
use crate::timefmt::is_fix_millis;
use crate::{APPL_VER_FIX50SP2, TARGET_COMP_ID};

/// `CancelOrdersOnDisconnect` (8013). `Profile` (`Y`) is the setting Coinbase recommends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOnDisconnect {
    /// Cancel orders placed on this session (`S`).
    Session,
    /// Cancel every order on the profile (`Y`).
    Profile,
}

impl CancelOnDisconnect {
    pub fn as_wire(self) -> char {
        match self {
            CancelOnDisconnect::Session => 'S',
            CancelOnDisconnect::Profile => 'Y',
        }
    }
}

type HmacSha256 = Hmac<Sha256>;

/// API key, base64 secret, and passphrase. Debug output redacts the secret and passphrase.
#[derive(Clone)]
pub struct Credentials {
    pub api_key: String,
    secret: String,
    passphrase: String,
}

impl Credentials {
    pub fn new(
        api_key: impl Into<String>,
        secret_b64: impl Into<String>,
        passphrase: impl Into<String>,
    ) -> Result<Self> {
        let api_key = api_key.into();
        let secret = secret_b64.into();
        let passphrase = passphrase.into();
        if api_key.is_empty() || passphrase.is_empty() {
            return Err(Error::auth("api key and passphrase are required"));
        }
        STANDARD
            .decode(secret.trim())
            .map_err(|_| Error::auth("API secret is not valid base64"))?;
        Ok(Self {
            api_key,
            secret: secret.trim().to_string(),
            passphrase,
        })
    }

    /// `COINBASE_API_KEY`, `COINBASE_API_SECRET`, `COINBASE_API_PASSPHRASE`.
    pub fn from_env() -> Result<Self> {
        fn var(name: &str) -> Result<String> {
            std::env::var(name).map_err(|_| Error::auth(format!("{name} is not set")))
        }
        Self::new(
            var("COINBASE_API_KEY")?,
            var("COINBASE_API_SECRET")?,
            var("COINBASE_API_PASSPHRASE")?,
        )
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub fn passphrase(&self) -> &str {
        &self.passphrase
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("api_key", &self.api_key)
            .field("secret", &"***")
            .field("passphrase", &"***")
            .finish()
    }
}

/// Base64 HMAC signature for a logon. `seq` is the decimal sequence number with no leading zeros.
pub fn sign_logon(
    secret_b64: &str,
    sending_time: &str,
    msg_type: &str,
    seq: &str,
    sender: &str,
    target: &str,
    passphrase: &str,
) -> Result<String> {
    if !is_fix_millis(sending_time) {
        return Err(Error::auth(format!(
            "SendingTime `{sending_time}` must be YYYYMMDD-HH:MM:SS.sss"
        )));
    }
    if seq.starts_with('0') {
        return Err(Error::auth("MsgSeqNum must not have a leading zero"));
    }
    let key = STANDARD
        .decode(secret_b64.trim())
        .map_err(|_| Error::auth("API secret is not valid base64"))?;
    let parts = [sending_time, msg_type, seq, sender, target, passphrase];
    let mut prehash = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if part.bytes().any(|b| b == SOH) {
            return Err(Error::auth("a signed logon field contains SOH"));
        }
        if i > 0 {
            prehash.push(SOH);
        }
        prehash.extend_from_slice(part.as_bytes());
    }
    let mut mac =
        HmacSha256::new_from_slice(&key).map_err(|_| Error::auth("HMAC key was rejected"))?;
    mac.update(&prehash);
    Ok(STANDARD.encode(mac.finalize().into_bytes()))
}

pub(crate) struct LogonBody {
    pub sending_time: String,
    pub message: AppMessage,
}

pub(crate) fn build_logon(
    creds: &Credentials,
    sending_time: &str,
    heartbeat_secs: u64,
    reset_seq: bool,
    cancel: Option<CancelOnDisconnect>,
    drop_copy: bool,
) -> Result<LogonBody> {
    let signature = sign_logon(
        &creds.secret,
        sending_time,
        "A",
        "1",
        &creds.api_key,
        TARGET_COMP_ID,
        &creds.passphrase,
    )?;
    let mut fields = vec![
        (ENCRYPT_METHOD, "0".into()),
        (HEART_BT_INT, heartbeat_secs.to_string()),
        (RESET_SEQ_NUM_FLAG, if reset_seq { "Y" } else { "N" }.into()),
        (USERNAME, creds.api_key.clone()),
        (PASSWORD, creds.passphrase.clone()),
        (RAW_DATA_LENGTH, signature.len().to_string()),
        (RAW_DATA, signature),
        (DEFAULT_APPL_VER_ID, APPL_VER_FIX50SP2.into()),
    ];
    if let Some(mode) = cancel {
        if drop_copy {
            return Err(Error::rejected(
                "drop copy sessions do not set CancelOrdersOnDisconnect",
            ));
        }
        fields.push((CANCEL_ORDERS_ON_DISCONNECT, mode.as_wire().into()));
    }
    if drop_copy {
        fields.push((DROP_COPY_FLAG, "Y".into()));
    }
    Ok(LogonBody {
        sending_time: sending_time.to_string(),
        message: AppMessage::new("A", fields),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
    const SIGN: &str = "Zopymq/hk+2+c+lurDxLG8x0uLGGEefRXb2sLDviEnM=";

    #[test]
    fn known_vector() {
        let sign = sign_logon(
            SECRET,
            "20230822-20:43:30.000",
            "A",
            "1",
            "test-key",
            "Coinbase",
            "passphrase",
        )
        .unwrap();
        assert_eq!(sign, SIGN);
        assert_eq!(sign.len(), 44);
        assert!(
            sign_logon(
                SECRET,
                "20230822-20:43:30",
                "A",
                "1",
                "test-key",
                "Coinbase",
                "passphrase"
            )
            .is_err()
        );
        assert!(
            sign_logon(
                SECRET,
                "20230822-20:43:30.000",
                "A",
                "01",
                "test-key",
                "Coinbase",
                "passphrase"
            )
            .is_err()
        );
    }

    #[test]
    fn logon_body_uses_that_signature() {
        let creds = Credentials::new("test-key", SECRET, "passphrase").unwrap();
        let body = build_logon(
            &creds,
            "20230822-20:43:30.000",
            10,
            true,
            Some(CancelOnDisconnect::Profile),
            false,
        )
        .unwrap();
        assert_eq!(
            body.message
                .fields
                .iter()
                .find(|(t, _)| *t == 96)
                .unwrap()
                .1,
            SIGN
        );
        assert_eq!(
            body.message
                .fields
                .iter()
                .find(|(t, _)| *t == 95)
                .unwrap()
                .1,
            "44"
        );
        assert_eq!(
            body.message
                .fields
                .iter()
                .find(|(t, _)| *t == 8013)
                .unwrap()
                .1,
            "Y"
        );
        let format = format!("{:?}", creds);
        assert!(format.contains("***"));
        assert!(!format.contains(SECRET));
        assert!(format.contains("test-key"));
    }
}

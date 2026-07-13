use crate::settings::AppChannel;
use serde::Serialize;

const PRODUCTION_TARGET: &str = "com.lingcang.immersivereading/deepseek-api-key";
const QA_TARGET: &str = "com.lingcang.immersivereading.qa/deepseek-api-key";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStatus {
    pub configured: bool,
    pub target: String,
    pub last_verified_at: Option<String>,
}

pub fn deepseek_target(channel: &AppChannel) -> &'static str {
    match channel {
        AppChannel::Production => PRODUCTION_TARGET,
        AppChannel::Qa(_) => QA_TARGET,
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn last_error(operation: &str) -> String {
    format!("{operation} failed: {}", std::io::Error::last_os_error())
}

#[cfg(windows)]
fn read_secret(target: &str) -> Result<Option<Vec<u8>>, String> {
    use windows_sys::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let target = wide(target);
    let mut raw: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: target is a live, NUL-terminated UTF-16 buffer; raw is a valid
    // out-pointer. A successful allocation is copied before CredFree.
    let succeeded = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) };
    if succeeded == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(1168) {
            return Ok(None);
        }
        return Err(format!("Credential read failed: {error}"));
    }
    if raw.is_null() {
        return Err("Credential read returned a null record".to_string());
    }
    // SAFETY: CredReadW returned a valid CREDENTIALW. The blob belongs to that
    // record and remains valid until CredFree below; from_raw_parts is used only
    // for an immediate owned copy.
    let value = unsafe {
        let credential = &*raw;
        if credential.CredentialBlobSize > 0 && credential.CredentialBlob.is_null() {
            CredFree(raw.cast());
            return Err("Credential blob was null".to_string());
        }
        let bytes = if credential.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
            .to_vec()
        };
        CredFree(raw.cast());
        bytes
    };
    Ok(Some(value))
}

#[cfg(windows)]
fn write_secret(target: &str, value: &[u8]) -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };

    if value.is_empty() {
        return Err("API key cannot be empty".to_string());
    }
    if value.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return Err("API key exceeds Credential Manager limit".to_string());
    }
    let mut target = wide(target);
    let mut user_name = wide("default");
    let mut blob = value.to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: user_name.as_mut_ptr(),
        ..Default::default()
    };
    // SAFETY: all pointers in credential reference live buffers for the whole
    // call; sizes match the byte buffer and unused fields are null/default.
    let succeeded = unsafe { CredWriteW(&credential, 0) };
    blob.fill(0);
    if succeeded == 0 {
        return Err(last_error("Credential write"));
    }
    Ok(())
}

#[cfg(windows)]
fn delete_secret(target: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};

    let target = wide(target);
    // SAFETY: target is a live, NUL-terminated UTF-16 buffer and the flags and
    // credential type follow the CredDeleteW contract.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(1168) {
            return Err(format!("Credential delete failed: {error}"));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn read_secret(_target: &str) -> Result<Option<Vec<u8>>, String> {
    Err("Windows Credential Manager is unavailable".to_string())
}

#[cfg(not(windows))]
fn write_secret(_target: &str, _value: &[u8]) -> Result<(), String> {
    Err("Windows Credential Manager is unavailable".to_string())
}

#[cfg(not(windows))]
fn delete_secret(_target: &str) -> Result<(), String> {
    Err("Windows Credential Manager is unavailable".to_string())
}

pub fn deepseek_status(channel: &AppChannel) -> Result<SecretStatus, String> {
    let target = deepseek_target(channel);
    let configured = match read_secret(target)? {
        Some(mut secret) => {
            secret.fill(0);
            true
        }
        None => false,
    };
    Ok(SecretStatus {
        configured,
        target: target.to_string(),
        last_verified_at: configured.then(|| chrono::Utc::now().to_rfc3339()),
    })
}

pub(crate) fn deepseek_api_key(channel: &AppChannel) -> Result<Option<String>, String> {
    let Some(mut secret) = read_secret(deepseek_target(channel))? else {
        return Ok(None);
    };
    let decoded = String::from_utf8(secret.clone())
        .map_err(|_| "Credential value was not valid UTF-8".to_string());
    secret.fill(0);
    decoded.map(Some)
}

pub fn set_deepseek_api_key(channel: &AppChannel, api_key: &str) -> Result<SecretStatus, String> {
    let target = deepseek_target(channel);
    write_secret(target, api_key.as_bytes())?;
    deepseek_status(channel)
}

pub fn delete_deepseek_api_key(channel: &AppChannel) -> Result<SecretStatus, String> {
    delete_secret(deepseek_target(channel))?;
    deepseek_status(channel)
}

#[cfg(test)]
mod tests {
    use super::{deepseek_target, SecretStatus, PRODUCTION_TARGET, QA_TARGET};
    use crate::settings::AppChannel;

    #[test]
    fn production_and_qa_targets_are_isolated() {
        assert_eq!(deepseek_target(&AppChannel::Production), PRODUCTION_TARGET);
        assert_eq!(
            deepseek_target(&AppChannel::Qa("run-1".to_string())),
            QA_TARGET
        );
        assert_ne!(PRODUCTION_TARGET, QA_TARGET);
    }

    #[test]
    fn secret_status_serialization_never_has_a_secret_field() {
        let status = SecretStatus {
            configured: true,
            target: QA_TARGET.to_string(),
            last_verified_at: None,
        };
        let json = serde_json::to_string(&status).expect("status must serialize");

        assert!(!json.contains("apiKey"));
        assert!(!json.contains("secret"));
        assert!(!json.contains("credentialBlob"));
    }
}

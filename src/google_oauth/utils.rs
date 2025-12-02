use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

/// Insert the `email` claim into the token payload from the embedded `id_token`, if available.
pub fn attach_email_from_id_token(token_value: &mut Value) {
    let email = token_value
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(|id_token| id_token.split('.').nth(1))
        .and_then(|payload_b64| URL_SAFE_NO_PAD.decode(payload_b64).ok())
        .and_then(|decoded| serde_json::from_slice::<Value>(&decoded).ok())
        .and_then(|payload| {
            payload
                .get("email")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });

    if let (Some(obj), Some(email)) = (token_value.as_object_mut(), email) {
        obj.insert("email".to_string(), Value::String(email));
    }
}

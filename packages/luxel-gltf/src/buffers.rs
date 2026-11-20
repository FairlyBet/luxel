//! Resolve every glTF `buffer` to its raw bytes.
//!
//! A buffer's data lives in one of three places (glTF 2.0 §3.6.1.2): the GLB
//! binary chunk (`Source::Bin`), a `data:` URI, or an external file referenced
//! by URI. External buffers are all requested at once, through the load's
//! fetcher.

use base64::Engine;
use wasm_bindgen::prelude::*;

use crate::fetch::{Load, resolve_url};

/// One `Vec<u8>` per buffer of the glTF `json`, in index order. Read from
/// the JSON, before the document is validated (Draco and meshopt files are
/// only valid once decoded).
pub async fn resolve_buffers(
    json: &serde_json::Value,
    glb_blob: Option<Vec<u8>>,
    load: &Load,
) -> Result<Vec<Vec<u8>>, JsValue> {
    let mut blob = glb_blob;
    let list = json["buffers"].as_array().map_or(&[][..], Vec::as_slice);
    // Every external one requested first, then each awaited in turn.
    let mut external = Vec::new();
    for buffer in list {
        external.push(match buffer["uri"].as_str() {
            Some(uri) if !uri.starts_with("data:") && !is_fallback(buffer) => {
                let abs = resolve_url(&load.base, uri)?;
                Some(load.fetcher.start_bytes(&abs))
            }
            _ => None,
        });
    }
    let mut out = Vec::with_capacity(list.len());
    for (buffer, fetch) in list.iter().zip(external) {
        let length = usize::try_from(buffer["byteLength"].as_u64().unwrap_or(0))
            .map_err(|_| JsValue::from_str("buffer: byteLength too large"))?;
        if is_fallback(buffer) {
            // Zeros the meshopt decode fills: a length the page can't hold
            // is an error, not an allocation abort.
            let mut zeros = Vec::new();
            zeros
                .try_reserve_exact(length)
                .map_err(|_| JsValue::from_str("buffer: byteLength too large"))?;
            zeros.resize(length, 0);
            out.push(zeros);
            continue;
        }
        let data = match buffer["uri"].as_str() {
            // No URI: the GLB BIN chunk (the first buffer).
            None => blob.take().ok_or_else(|| {
                JsValue::from_str("glTF references a GLB buffer but none present")
            })?,
            Some(uri) => match (decode_data_uri(uri), fetch) {
                (Some(bytes), _) => bytes?,
                (None, Some(fetch)) => fetch.await?,
                (None, None) => return Err("buffer: not requested".into()),
            },
        };
        if data.len() < length {
            crate::fetch::warn(format!(
                "buffer {}: {} bytes where byteLength says {length}",
                out.len(),
                data.len()
            ));
        }
        out.push(data);
    }
    Ok(out)
}

/// The meshopt extension under both its names (EXT first, KHR ratified).
pub(crate) const MESHOPT_EXTENSIONS: [&str; 2] =
    ["EXT_meshopt_compression", "KHR_meshopt_compression"];

/// A bufferView's buffer index and byte range, read from its JSON (before a
/// document exists); None when they overflow.
pub(crate) fn json_view(view: &serde_json::Value) -> Option<(usize, std::ops::Range<usize>)> {
    let buffer = usize::try_from(view["buffer"].as_u64()?).ok()?;
    let start = usize::try_from(view["byteOffset"].as_u64().unwrap_or(0)).ok()?;
    let len = usize::try_from(view["byteLength"].as_u64()?).ok()?;
    Some((buffer, start..start.checked_add(len)?))
}

// A meshopt fallback buffer is what the compressed bufferViews decode into:
// zeroed, never fetched (its URI, if any, is for loaders without the
// extension).
fn is_fallback(buffer: &serde_json::Value) -> bool {
    MESHOPT_EXTENSIONS
        .iter()
        .filter_map(|e| buffer.get("extensions")?.get(e))
        .any(|v| v.get("fallback").and_then(serde_json::Value::as_bool) == Some(true))
}

/// Decode a `data:` URI's payload. Returns `None` if `uri` isn't a data URI.
/// Handles both `;base64` and (rarely, for buffers) percent-less inline text.
pub fn decode_data_uri(uri: &str) -> Option<Result<Vec<u8>, JsValue>> {
    let rest = uri.strip_prefix("data:")?;
    let comma = match rest.find(',') {
        Some(c) => c,
        None => return Some(Err(JsValue::from_str("malformed data URI: no comma"))),
    };
    let meta = &rest[..comma];
    let payload = &rest[comma + 1..];
    if meta.contains(";base64") {
        Some(
            base64::engine::general_purpose::STANDARD
                .decode(payload)
                .map_err(|e| JsValue::from_str(&format!("data URI base64 decode: {e}"))),
        )
    } else {
        Some(Ok(percent_decode(payload)))
    }
}

// `%XX` escapes decoded (RFC 2397: a non-base64 payload is URL-encoded);
// a malformed one is kept as it is.
fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// The MIME type declared in a `data:` image URI, if any (`data:image/png;base64,…`).
pub fn data_uri_mime(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let meta = &rest[..comma];
    let mime = meta.split(';').next().unwrap_or("");
    (!mime.is_empty()).then(|| mime.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_data_uri() {
        // "AAAA" base64 → three zero bytes.
        let got = decode_data_uri("data:application/octet-stream;base64,AAAA")
            .expect("is data URI")
            .expect("decodes");
        assert_eq!(got, vec![0, 0, 0]);
    }

    #[test]
    fn decodes_percent_encoded_data_uri() {
        let got = decode_data_uri("data:,a%20b%2Cc%zz").unwrap().unwrap();
        assert_eq!(got, b"a b,c%zz");
    }

    #[test]
    fn non_data_uri_is_none() {
        assert!(decode_data_uri("buffer.bin").is_none());
        assert!(decode_data_uri("https://example.com/x.bin").is_none());
    }

    #[test]
    fn reads_image_mime() {
        assert_eq!(
            data_uri_mime("data:image/png;base64,iVBOR"),
            Some("image/png".to_string())
        );
        assert_eq!(data_uri_mime("data:,plain"), None);
        assert_eq!(data_uri_mime("not-a-data-uri"), None);
    }
}

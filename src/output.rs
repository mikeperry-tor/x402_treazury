//! Typed, bounded inline image results. No URL fetching, transcoding or artifact storage.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod projection;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImageLimits {
    pub max_image_bytes: usize,
    pub max_total_bytes: usize,
    pub max_images: usize,
}
impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_image_bytes: 4 * 1024 * 1024,
            max_total_bytes: 8 * 1024 * 1024,
            max_images: 4,
        }
    }
}
impl ImageLimits {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.max_image_bytes > 0 && self.max_total_bytes > 0 && self.max_images > 0,
            "image_limits values must be positive"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseMapping {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omit: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageField {
    pub pointer: String,
    #[serde(default)]
    pub encoding: ImageEncoding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_pointer: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageEncoding {
    #[default]
    Base64,
    DataUri,
}
fn valid_pointer(pointer: &str) -> bool {
    !pointer.is_empty()
        && pointer.starts_with('/')
        && pointer
            .split('~')
            .skip(1)
            .all(|s| s.starts_with('0') || s.starts_with('1'))
}
impl ResponseMapping {
    pub fn validate(&self, limits: &ImageLimits) -> Result<()> {
        ensure!(
            !self.images.is_empty() || !self.omit.is_empty(),
            "response mapping requires images or omitted fields"
        );
        for pointer in &self.omit {
            ensure!(valid_pointer(pointer), "invalid JSON omission pointer");
        }
        bound(
            self.images.len(),
            limits.max_images,
            "image_limits.max_images",
        )?;
        for (i, field) in self.images.iter().enumerate() {
            ensure!(
                valid_pointer(&field.pointer),
                "image pointer must be a non-root JSON pointer"
            );
            ensure!(
                field.mime_type.is_none() || field.mime_pointer.is_none(),
                "choose mime_type or mime_pointer, not both"
            );
            if let Some(mime) = &field.mime_type {
                ensure!(supported(mime), "unsupported image MIME type");
            }
            if let Some(pointer) = &field.mime_pointer {
                ensure!(valid_pointer(pointer), "invalid MIME JSON pointer");
            }
            if matches!(field.encoding, ImageEncoding::Base64) {
                ensure!(
                    field.mime_type.is_some() || field.mime_pointer.is_some(),
                    "base64 image needs mime_type or mime_pointer"
                );
            }
            for other in &self.images[..i] {
                ensure!(
                    field.pointer != other.pointer
                        && !field.pointer.starts_with(&format!("{}/", other.pointer))
                        && !other.pointer.starts_with(&format!("{}/", field.pointer)),
                    "image pointers must not overlap"
                );
            }
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct HttpOutput {
    pub advisories: Vec<String>,
    pub bytes: Vec<u8>,
    pub mime_type: Option<String>,
    /// Target authorization was submitted; relay fees do not set this flag.
    pub paid_submission: bool,
    /// Separate possible liability for acquiring this response through a relay.
    pub relay_payment_may_have_occurred: bool,
}
#[derive(Debug)]
pub struct InlineImage {
    pub data: String,
    pub mime_type: String,
}
#[derive(Debug)]
pub struct ToolOutput {
    /// Aggregate operation evidence for subsequent fallible output consumers.
    pub payment_may_have_occurred: bool,
    pub advisories: Vec<String>,
    pub text: String,
    pub images: Vec<InlineImage>,
}
impl ToolOutput {
    pub fn text(text: String) -> Self {
        Self {
            payment_may_have_occurred: false,
            text,
            advisories: vec![],
            images: vec![],
        }
    }
    pub fn into_text(self) -> Result<String> {
        crate::payment::with_payment_context(
            self.payment_may_have_occurred,
            if self.images.is_empty() {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "image result requires the typed MCP result interface"
                ))
            },
        )?;
        Ok(if self.advisories.is_empty() {
            self.text
        } else {
            format!("{}\n{}", self.text, self.advisories.join("\n"))
        })
    }
    pub fn into_content(self) -> Vec<rmcp::model::ContentBlock> {
        let mut content = vec![rmcp::model::ContentBlock::text(self.text)];
        content.extend(
            self.images
                .into_iter()
                .map(|i| rmcp::model::ContentBlock::image(i.data, i.mime_type)),
        );
        content.extend(
            self.advisories
                .into_iter()
                .map(rmcp::model::ContentBlock::text),
        );
        content
    }
}
fn supported(mime: &str) -> bool {
    matches!(mime, "image/png" | "image/jpeg" | "image/webp")
}
fn image_signature_matches(bytes: &[u8], mime: &str) -> bool {
    match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        _ => false,
    }
}
pub(crate) fn text_mime(mime: &str) -> bool {
    mime.is_empty()
        || mime.starts_with("text/")
        || mime == "application/json"
        || mime.ends_with("+json")
        || matches!(
            mime,
            "application/xml" | "application/javascript" | "application/x-www-form-urlencoded"
        )
        || (mime.starts_with("application/") && mime.ends_with("+xml"))
}
fn bound(actual: usize, limit: usize, setting: &'static str) -> Result<()> {
    if actual > limit {
        tracing::warn!(
            resource = "inline image output",
            setting,
            limit,
            "output limit exceeded; no partial content returned"
        );
        anyhow::bail!(
            "inline image output exceeds {setting}={limit}; content rejected, no partial content returned. Ask the operator to raise {setting} if appropriate."
        );
    }
    Ok(())
}
fn image(bytes: &[u8], mime: &str, limits: &ImageLimits, total: &mut usize) -> Result<InlineImage> {
    bound(
        bytes.len(),
        limits.max_image_bytes,
        "image_limits.max_image_bytes",
    )?;
    *total = total
        .checked_add(bytes.len())
        .context("inline image size overflow")?;
    bound(
        *total,
        limits.max_total_bytes,
        "image_limits.max_total_bytes",
    )?;
    ensure!(
        image_signature_matches(bytes, mime),
        "unsupported image format or image signature does not match MIME type"
    );
    Ok(InlineImage {
        data: STANDARD.encode(bytes),
        mime_type: mime.into(),
    })
}
impl HttpOutput {
    pub(crate) fn is_valid_unmapped(&self) -> bool {
        let limits = ImageLimits::default();
        let mime = self.mime_type.as_deref().unwrap_or("");
        if supported(mime) {
            return image_signature_matches(&self.bytes, mime)
                && self.bytes.len() <= limits.max_image_bytes
                && self.bytes.len() <= limits.max_total_bytes;
        }
        text_mime(mime) && std::str::from_utf8(&self.bytes).is_ok()
    }
    pub fn render(
        mut self,
        mapping: Option<&ResponseMapping>,
        limits: &ImageLimits,
    ) -> Result<ToolOutput> {
        let paid = self.paid_submission || self.relay_payment_may_have_occurred;
        let advisories = std::mem::take(&mut self.advisories);
        crate::payment::with_payment_context(
            paid,
            self.render_inner(mapping, limits)
                .map(|mut output| {
                    output.advisories.extend(advisories);
                    output.payment_may_have_occurred = paid;
                    output
                })
                .inspect_err(|_| {
                    tracing::warn!("API output conversion rejected; no partial result returned");
                }),
        )
    }
    fn render_inner(
        self,
        mapping: Option<&ResponseMapping>,
        limits: &ImageLimits,
    ) -> Result<ToolOutput> {
        limits.validate()?;
        let mut total: usize = 0;
        let mime = self.mime_type.as_deref().unwrap_or("");
        if supported(mime) {
            bound(1, limits.max_images, "image_limits.max_images")?;
            let attachment = image(&self.bytes, mime, limits, &mut total)?;
            return Ok(ToolOutput {
                payment_may_have_occurred: false,
                advisories: vec![],
                text: format!(
                    "Inline image attachment 1: {mime}, {} bytes.",
                    self.bytes.len()
                ),
                images: vec![attachment],
            });
        }
        ensure!(
            text_mime(mime),
            "unsupported response Content-Type; binary output requires a supported image type or future artifact storage"
        );
        let text = String::from_utf8(self.bytes).context(
            "API response is not valid UTF-8 text; binary data was not converted to text",
        )?;
        let Some(mapping) = mapping else {
            return Ok(ToolOutput::text(text));
        };
        mapped_output(&text, mapping, limits)
    }
}

fn mapped_output(
    text: &str,
    mapping: &ResponseMapping,
    limits: &ImageLimits,
) -> Result<ToolOutput> {
    let mut total: usize = 0;
    mapping.validate(limits)?;
    let mut doc: Value = serde_json::from_str(text).context("response mapping requires JSON")?;
    let mut images = Vec::new();
    // Read every field before replacing any; MIME pointers may refer to other fields.
    for field in &mapping.images {
        let (mime, data) = image_data(field, &doc)?;
        ensure!(supported(mime), "unsupported image MIME type");
        // Bound allocations before decoding; canonical base64's decoded length is exact.
        ensure!(data.len().is_multiple_of(4), "invalid base64 image length");
        let padding = data
            .as_bytes()
            .iter()
            .rev()
            .take_while(|&&b| b == b'=')
            .count();
        ensure!(padding <= 2, "invalid base64 image padding");
        let length = (data.len() / 4 * 3).saturating_sub(padding);
        bound(
            length,
            limits.max_image_bytes,
            "image_limits.max_image_bytes",
        )?;
        bound(
            total
                .checked_add(length)
                .context("inline image size overflow")?,
            limits.max_total_bytes,
            "image_limits.max_total_bytes",
        )?;
        let bytes = STANDARD.decode(data).context("invalid base64 image data")?;
        images.push(image(&bytes, mime, limits, &mut total)?);
    }
    for (index, field) in mapping.images.iter().enumerate() {
        *doc.pointer_mut(&field.pointer)
            .expect("validated image pointer") =
            json!({"treazury_attachment": index + 1, "mime_type": images[index].mime_type});
    }
    let advisories = projection::apply(&mut doc, &mapping.omit);
    let text = serde_json::to_string(&doc)?;
    Ok(ToolOutput {
        payment_may_have_occurred: false,
        advisories,
        text: if images.is_empty() {
            text
        } else {
            format!(
                "Image payloads extracted into numbered MCP image attachments; metadata follows:\n{text}"
            )
        },
        images,
    })
}
fn image_data<'a>(field: &'a ImageField, doc: &'a Value) -> Result<(&'a str, &'a str)> {
    let encoded = doc
        .pointer(&field.pointer)
        .and_then(Value::as_str)
        .context("mapped image field is missing or not a string")?;
    let declared = field.mime_type.as_deref().or_else(|| {
        field
            .mime_pointer
            .as_ref()
            .and_then(|p| doc.pointer(p))
            .and_then(Value::as_str)
    });
    let result = match field.encoding {
        ImageEncoding::Base64 => (
            declared.context("mapped image MIME field is missing or not a string")?,
            encoded,
        ),
        ImageEncoding::DataUri => {
            let (header, data) = encoded.split_once(',').context("invalid image data URI")?;
            let mime = header
                .strip_prefix("data:")
                .and_then(|s| s.strip_suffix(";base64"))
                .context("image data URI must use base64")?;
            if let Some(declared) = declared {
                ensure!(mime == declared, "image data URI MIME mismatch");
            }
            if field.mime_pointer.is_some() {
                ensure!(declared.is_some(), "mapped image MIME field is missing");
            }
            (mime, data)
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmapped_promotion_and_default_rendering_accept_the_same_output() {
        let cases: &[(Option<&str>, &[u8], bool)] = &[
            (None, b"plain text", true),
            (Some("text/plain"), b"hello", true),
            (Some("application/json"), b"{}", true),
            (Some("application/problem+json"), b"{}", true),
            (Some("application/xml"), b"<xml/>", true),
            (Some("application/atom+xml"), b"<xml/>", true),
            (Some("application/javascript"), b"let x = 1;", true),
            (Some("application/x-www-form-urlencoded"), b"x=1", true),
            (Some("image/png"), b"\x89PNG\r\n\x1a\n", true),
            (Some("image/jpeg"), b"\xff\xd8\xff", true),
            (Some("image/webp"), b"RIFF\0\0\0\0WEBP", true),
            (Some("image/png"), b"wrong signature", false),
            (Some("image/jpeg"), b"\x89PNG\r\n\x1a\n", false),
            (Some("image/webp"), b"RIFF", false),
            (Some("application/pdf"), b"%PDF", false),
            (Some("image/svg+xml"), b"<svg/>", false),
            (Some("text/plain"), b"\xff", false),
            (None, b"\xff", false),
        ];
        let limits = ImageLimits::default();
        for &(mime, bytes, expected) in cases {
            let output = HttpOutput {
                advisories: vec![],
                bytes: bytes.to_vec(),
                mime_type: mime.map(str::to_owned),
                paid_submission: false,
                relay_payment_may_have_occurred: false,
            };
            assert_eq!(output.is_valid_unmapped(), expected, "{mime:?}");
            assert_eq!(output.render(None, &limits).is_ok(), expected, "{mime:?}");
        }
        for (size, expected) in [
            (limits.max_image_bytes, true),
            (limits.max_image_bytes + 1, false),
        ] {
            let mut bytes = vec![0; size];
            bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
            let output = HttpOutput {
                advisories: vec![],
                bytes,
                mime_type: Some("image/png".into()),
                paid_submission: false,
                relay_payment_may_have_occurred: false,
            };
            assert_eq!(output.is_valid_unmapped(), expected);
            assert_eq!(output.render(None, &limits).is_ok(), expected);
        }
    }
}

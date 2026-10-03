//! Typed, bounded inline image results. No URL fetching, transcoding or artifact storage.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    pub images: Vec<ImageField>,
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
        ensure!(!self.images.is_empty(), "response mapping requires images");
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
    pub bytes: Vec<u8>,
    pub mime_type: Option<String>,
    pub paid_submission: bool,
}
#[derive(Debug)]
pub struct InlineImage {
    pub data: String,
    pub mime_type: String,
}
#[derive(Debug)]
pub struct ToolOutput {
    pub text: String,
    pub images: Vec<InlineImage>,
}
impl ToolOutput {
    pub fn text(text: String) -> Self {
        Self {
            text,
            images: vec![],
        }
    }
    pub fn into_text(self) -> Result<String> {
        ensure!(
            self.images.is_empty(),
            "image result requires the typed MCP result interface"
        );
        Ok(self.text)
    }
    pub fn into_content(self) -> Vec<rmcp::model::ContentBlock> {
        let mut content = vec![rmcp::model::ContentBlock::text(self.text)];
        content.extend(
            self.images
                .into_iter()
                .map(|i| rmcp::model::ContentBlock::image(i.data, i.mime_type)),
        );
        content
    }
}
fn supported(mime: &str) -> bool {
    matches!(mime, "image/png" | "image/jpeg" | "image/webp")
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
    let valid = match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        _ => false,
    };
    ensure!(
        valid,
        "unsupported image format or image signature does not match MIME type"
    );
    Ok(InlineImage {
        data: STANDARD.encode(bytes),
        mime_type: mime.into(),
    })
}
impl HttpOutput {
    pub fn render(
        self,
        mapping: Option<&ResponseMapping>,
        limits: &ImageLimits,
    ) -> Result<ToolOutput> {
        let paid = self.paid_submission;
        self.render_inner(mapping, limits).map_err(|error| {
            tracing::warn!("API output conversion rejected; no partial result returned");
            if paid { error.context("API output unavailable; a payment may already have settled. Do not automatically retry a paid request") }
            else { error }
        })
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
                text: format!(
                    "Inline image attachment 1: {mime}, {} bytes.",
                    self.bytes.len()
                ),
                images: vec![attachment],
            });
        }
        ensure!(
            mime.is_empty()
                || mime.starts_with("text/")
                || mime == "application/json"
                || mime.ends_with("+json")
                || matches!(
                    mime,
                    "application/xml"
                        | "application/javascript"
                        | "application/x-www-form-urlencoded"
                )
                || (mime.starts_with("application/") && mime.ends_with("+xml")),
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
    let mut doc: Value =
        serde_json::from_str(text).context("image response mapping requires JSON")?;
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
    Ok(ToolOutput {
        text: format!(
            "Image payloads extracted into numbered MCP image attachments; metadata follows:\n{}",
            serde_json::to_string(&doc)?
        ),
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

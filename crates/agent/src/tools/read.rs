//! `read`: file contents with offset/limit and head truncation. Images come back as attachments.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::resolve_to_cwd;
use super::truncate::{format_size, truncate_head, TruncatedBy, TruncationOptions, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};
use crate::types::ContentPart;

pub struct ReadTool {
    cwd: PathBuf,
}

impl ReadTool {
    pub fn new(cwd: PathBuf) -> Self {
        ReadTool { cwd }
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp, tiff). Images are sent as attachments, \
         scaled down to fit 2000x2000 pixels. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). \
         Use offset/limit for large files. When you need the full file, continue with offset until complete."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to read (relative or absolute)" },
                "offset": { "type": "number", "description": "Line number to start reading from (1-indexed)" },
                "limit": { "type": "number", "description": "Maximum number of lines to read" }
            },
            "required": ["path"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let path = args["path"].as_str().ok_or("path is required")?.to_string();
        let offset = args["offset"].as_f64().map(|n| n as usize);
        let limit = args["limit"].as_f64().map(|n| n as usize);
        let absolute = resolve_to_cwd(&path, &self.cwd);

        let bytes = tokio::fs::read(&absolute).await.map_err(|e| ToolError(format!("Cannot read {path}: {e}")))?;
        if cancel.is_cancelled() {
            return Err("Operation aborted".into());
        }

        if let Some(mime) = crate::images::file_type(&bytes) {
            // As a model takes it, after pi's read: converted, turned upright, or scaled down
            // with a note saying so; one that cannot go is named with why, never attached.
            let prepared = tokio::task::spawn_blocking(move || crate::images::prepare(&bytes)).await.map_err(|e| ToolError(e.to_string()))?;
            let content = match prepared {
                Ok(image) => {
                    let mut text = format!("Read image file [{}]", image.mime_type);
                    if let Some(note) = &image.note {
                        text = format!("{text}\n{note}");
                    }
                    vec![ContentPart::text(text), ContentPart::Image { data: image.data, mime_type: image.mime_type }]
                }
                Err(why) => vec![ContentPart::text(format!("Read image file [{mime}]: {why}, so it is not attached."))],
            };
            return Ok(ToolResult { content, details: Value::Null, ..ToolResult::default() });
        }

        let text = String::from_utf8_lossy(&bytes).into_owned();
        let all_lines: Vec<&str> = text.split('\n').collect();
        let total_file_lines = all_lines.len();
        let start_line = offset.map(|o| o.saturating_sub(1)).unwrap_or(0);
        let start_display = start_line + 1;
        if start_line >= all_lines.len() {
            return Err(ToolError(format!("Offset {} is beyond end of file ({} lines total)", offset.unwrap_or(1), all_lines.len())));
        }

        let (selected, user_limited) = match limit {
            Some(limit) => {
                let end = (start_line + limit).min(all_lines.len());
                (all_lines[start_line..end].join("\n"), Some(end - start_line))
            }
            None => (all_lines[start_line..].join("\n"), None),
        };

        let truncation = truncate_head(&selected, TruncationOptions::default());
        let output = if truncation.first_line_exceeds_limit {
            let first_line_size = format_size(all_lines[start_line].len());
            format!(
                "[Line {start_display} is {first_line_size}, exceeds {} limit. Use bash: sed -n '{start_display}p' {path} | head -c {DEFAULT_MAX_BYTES}]",
                format_size(DEFAULT_MAX_BYTES)
            )
        } else if truncation.truncated {
            let end_display = start_display + truncation.output_lines - 1;
            let next_offset = end_display + 1;
            let mut out = truncation.content.clone();
            if truncation.truncated_by == Some(TruncatedBy::Lines) {
                out.push_str(&format!("\n\n[Showing lines {start_display}-{end_display} of {total_file_lines}. Use offset={next_offset} to continue.]"));
            } else {
                out.push_str(&format!(
                    "\n\n[Showing lines {start_display}-{end_display} of {total_file_lines} ({} limit). Use offset={next_offset} to continue.]",
                    format_size(DEFAULT_MAX_BYTES)
                ));
            }
            out
        } else if let Some(shown) = user_limited.filter(|shown| start_line + shown < all_lines.len()) {
            let remaining = all_lines.len() - (start_line + shown);
            let next_offset = start_line + shown + 1;
            format!("{}\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]", truncation.content)
        } else {
            truncation.content.clone()
        };

        let details = if truncation.truncated { json!({ "truncation": truncation, "max_lines": DEFAULT_MAX_LINES }) } else { Value::Null };
        Ok(ToolResult::text(output).with_details(details))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb};
    use std::sync::Arc;

    /// What reading a file with these bytes gives the model.
    async fn read(dir: &std::path::Path, name: &str, bytes: &[u8]) -> Vec<ContentPart> {
        std::fs::write(dir.join(name), bytes).unwrap();
        ReadTool::new(dir.to_path_buf()).execute("call-1", json!({ "path": name }), CancellationToken::new(), Arc::new(|_| {})).await.unwrap().content
    }

    fn picture(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 90])));
        let mut bytes = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut bytes), format).unwrap();
        bytes
    }

    fn text(parts: &[ContentPart]) -> &str {
        parts[0].as_text().unwrap()
    }

    #[tokio::test]
    async fn images_go_as_a_model_takes_them() {
        let dir = std::env::temp_dir().join(format!("lorca-read-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let small = read(&dir, "small.png", &picture(64, 48, ImageFormat::Png)).await;
        assert_eq!(text(&small), "Read image file [image/png]");
        assert!(matches!(&small[1], ContentPart::Image { mime_type, .. } if mime_type == "image/png"), "{small:?}");
        let bmp = read(&dir, "old.bmp", &picture(64, 48, ImageFormat::Bmp)).await;
        assert!(text(&bmp).contains("\n[Image converted from image/bmp to image/"), "{}", text(&bmp));
        assert!(matches!(&bmp[1], ContentPart::Image { .. }));
        let page = read(&dir, "page.png", &picture(2600, 100, ImageFormat::Png)).await;
        assert!(text(&page).ends_with("\n[Image: original 2600x100, displayed at 2000x77. Multiply coordinates by 1.30 to map to original image.]"), "{}", text(&page));
        let cut = read(&dir, "cut.png", &picture(64, 48, ImageFormat::Png)[..40]).await;
        assert_eq!(cut.len(), 1, "{cut:?}");
        assert!(text(&cut).starts_with("Read image file [image/png]: ") && text(&cut).ends_with(", so it is not attached."), "{}", text(&cut));
        let notes = read(&dir, "cars.txt", b"BMW and Audi\nVolvo").await;
        assert_eq!(notes, [ContentPart::text("BMW and Audi\nVolvo")], "a text file that starts with BM is text");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

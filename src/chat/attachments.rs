//! Private, content-addressed PNG drafts; wire messages carry identifiers only.
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::{Path, PathBuf};
pub const MAX_IMAGES: usize = 4;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
pub fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn path(dir: &Path, id: &str) -> PathBuf {
    dir.join("images").join(format!("{id}.png"))
}
pub fn save(dir: &Path, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err("Source image exceeds 64 MB.".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unsupported image.")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "Cannot read this image; use a PNG, JPEG or screenshot up to 8192×8192.")?;
    let mut png = Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "Cannot prepare image.")?;
    let png = png.into_inner();
    if png.len() > MAX_BYTES {
        return Err("Prepared image exceeds 8 MB.".into());
    }
    let id = format!("{:x}", Sha256::digest(&png));
    std::fs::create_dir_all(dir.join("images")).map_err(|_| "Cannot save image draft.")?;
    let p = path(dir, &id);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options
        .open(p)
        .and_then(|mut f| f.write_all(&png))
        .map_err(|_| "Cannot save image draft.")?;
    Ok(id)
}
pub fn load_draft(dir: &Path) -> Vec<String> {
    let ids: Vec<String> = std::fs::read(dir.join("draft-images.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    ids.into_iter()
        .filter(|id| valid_id(id) && path(dir, id).is_file())
        .take(MAX_IMAGES)
        .collect()
}
pub fn save_draft(dir: &Path, ids: &[String]) {
    if let Ok(bytes) = serde_json::to_vec(ids) {
        let _ = std::fs::write(dir.join("draft-images.json"), bytes);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_drafts_are_validated_and_durable() {
        let dir = std::env::temp_dir().join(format!("herdr-image-{}", super::super::fresh_id()));
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(12, 8)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let id = save(&dir, png.get_ref()).unwrap();
        assert!(valid_id(&id));
        assert!(save(&dir, b"not an image").is_err());
        save_draft(&dir, &[id.clone(), "../../elsewhere".into()]);
        assert_eq!(load_draft(&dir), vec![id.clone()]);
        assert_eq!(save(&dir, png.get_ref()).unwrap(), id);
        save_draft(&dir, &[]);
        assert!(load_draft(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

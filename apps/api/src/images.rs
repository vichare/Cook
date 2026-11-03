use std::io::{BufReader, Cursor};

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{
        header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
        HeaderMap, HeaderValue, Response, StatusCode,
    },
    Extension,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use image::{
    codecs::{
        jpeg::{JpegDecoder, JpegEncoder},
        png::PngDecoder,
        webp::WebPDecoder,
    },
    imageops::FilterType,
    DynamicImage, ImageBuffer, ImageDecoder, ImageFormat, Limits, Rgb, RgbImage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use url::Url;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    imports::{authenticate_token, record_token_audit, TokenPrincipal, SCOPE_RECIPES_READ},
    routes::{expected_recipe_version, CurrentUser},
    AppState,
};

pub const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_STORED_BYTES: usize = 2_500_000;
pub const MAX_PIXELS: u64 = 20_000_000;
pub const MAX_DIMENSION: u32 = 1_600;
const MAX_DECODER_DIMENSION: u32 = 10_000;
const MAX_DECODER_ALLOC: u64 = 128 * 1024 * 1024;

#[derive(Clone)]
pub struct ImageMetadataInput {
    pub alt_text: String,
    pub source_url: Option<String>,
    pub source_license: Option<String>,
    pub source_attribution: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageMetadataUpdate {
    alt_text: String,
    #[serde(default)]
    source_url: String,
    #[serde(default)]
    source_license: String,
    #[serde(default)]
    source_attribution: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeVersion {
    version: i64,
}

#[derive(Clone)]
pub struct ProcessedImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub sha256: [u8; 32],
    pub metadata: ImageMetadataInput,
}

pub fn upload_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_UPLOAD_BYTES + 64 * 1024)
}

pub async fn parse_image_multipart(mut multipart: Multipart) -> AppResult<ProcessedImage> {
    let mut image_bytes = None;
    let mut alt_text = None;
    let mut source_url = None;
    let mut source_license = None;
    let mut source_attribution = None;
    while let Some(field) = multipart.next_field().await.map_err(map_multipart_error)? {
        let name = field.name().unwrap_or_default().to_string();
        if name == "image" {
            let bytes = field.bytes().await.map_err(map_multipart_error)?;
            if bytes.len() > MAX_UPLOAD_BYTES {
                return Err(AppError::PayloadTooLarge(format!(
                    "image upload must be at most {MAX_UPLOAD_BYTES} bytes"
                )));
            }
            image_bytes = Some(bytes.to_vec());
            continue;
        }
        let value = field.text().await.map_err(map_multipart_error)?;
        match name.as_str() {
            "altText" => alt_text = Some(value),
            "sourceUrl" => source_url = Some(value),
            "sourceLicense" => source_license = Some(value),
            "sourceAttribution" => source_attribution = Some(value),
            _ => {}
        }
    }
    let metadata = validate_metadata(
        alt_text.as_deref().unwrap_or_default(),
        source_url.as_deref(),
        source_license.as_deref(),
        source_attribution.as_deref(),
    )?;
    process_image(
        image_bytes.ok_or_else(|| AppError::Validation("image file is required".into()))?,
        metadata,
    )
}

fn map_multipart_error(error: axum::extract::multipart::MultipartError) -> AppError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::PayloadTooLarge("image upload exceeds the request byte limit".into())
    } else {
        AppError::Validation("invalid multipart image upload".into())
    }
}

fn validate_metadata(
    alt_text: &str,
    source_url: Option<&str>,
    source_license: Option<&str>,
    source_attribution: Option<&str>,
) -> AppResult<ImageMetadataInput> {
    let alt_text = bounded_text("altText", alt_text, 300, false)?;
    let source_url = optional_bounded_text("sourceUrl", source_url, 2048)?;
    if let Some(value) = &source_url {
        let url = Url::parse(value)
            .map_err(|_| AppError::Validation("sourceUrl must be a valid URL".into()))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(AppError::Validation(
                "sourceUrl must use http or https and include a host".into(),
            ));
        }
    }
    Ok(ImageMetadataInput {
        alt_text,
        source_url,
        source_license: optional_bounded_text("sourceLicense", source_license, 120)?,
        source_attribution: optional_bounded_text("sourceAttribution", source_attribution, 500)?,
    })
}

fn process_image(bytes: Vec<u8>, metadata: ImageMetadataInput) -> AppResult<ProcessedImage> {
    let format = image::guess_format(&bytes)
        .map_err(|_| AppError::Validation("image must be JPEG, PNG, or WebP".into()))?;
    if !matches!(
        format,
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
    ) {
        return Err(AppError::Validation(
            "image must be JPEG, PNG, or WebP; SVG and GIF are not accepted".into(),
        ));
    }
    let cursor = Cursor::new(bytes);
    let image = match format {
        ImageFormat::Jpeg => decode_image(JpegDecoder::new(BufReader::new(cursor))),
        ImageFormat::Png => decode_image(PngDecoder::new(BufReader::new(cursor))),
        ImageFormat::WebP => decode_image(WebPDecoder::new(BufReader::new(cursor))),
        _ => unreachable!(),
    }?;
    let image = if image.width() > MAX_DIMENSION || image.height() > MAX_DIMENSION {
        image.resize(MAX_DIMENSION, MAX_DIMENSION, FilterType::Lanczos3)
    } else {
        image
    };
    let rgb = flatten_to_rgb(&image);
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 85)
        .encode_image(&DynamicImage::ImageRgb8(rgb))
        .map_err(|_| AppError::Validation("image could not be safely re-encoded".into()))?;
    if encoded.len() > MAX_STORED_BYTES {
        return Err(AppError::PayloadTooLarge(format!(
            "normalized image must be at most {MAX_STORED_BYTES} bytes"
        )));
    }
    Ok(ProcessedImage {
        width: image.width(),
        height: image.height(),
        sha256: Sha256::digest(&encoded).into(),
        bytes: encoded,
        metadata,
    })
}

fn decode_image<D>(decoder: Result<D, image::ImageError>) -> AppResult<DynamicImage>
where
    D: ImageDecoder + 'static,
{
    let mut decoder =
        decoder.map_err(|_| AppError::Validation("image data is malformed".into()))?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || width > MAX_DECODER_DIMENSION
        || height > MAX_DECODER_DIMENSION
    {
        return Err(AppError::PayloadTooLarge(format!(
            "decoded image must be at most {MAX_PIXELS} pixels"
        )));
    }
    let orientation = decoder
        .orientation()
        .map_err(|_| AppError::Validation("image orientation metadata is invalid".into()))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODER_DIMENSION);
    limits.max_image_height = Some(MAX_DECODER_DIMENSION);
    limits.max_alloc = Some(MAX_DECODER_ALLOC);
    decoder
        .set_limits(limits)
        .map_err(|_| AppError::PayloadTooLarge("image decoder limits exceeded".into()))?;
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|_| AppError::Validation("image data is malformed".into()))?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn flatten_to_rgb(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    ImageBuffer::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y).0;
        let alpha = u16::from(pixel[3]);
        let blend = |channel: u8| ((u16::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8;
        Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])])
    })
}

pub async fn upsert_image(
    transaction: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
    image: &ProcessedImage,
) -> AppResult<String> {
    let existing_version: Option<i64> =
        sqlx::query_scalar("SELECT version FROM recipe_images WHERE recipe_id = ?")
            .bind(recipe_id)
            .fetch_optional(&mut **transaction)
            .await?;
    let id = Uuid::new_v4().to_string();
    let version = existing_version.map_or(1, |value| value + 1);
    sqlx::query(
        "INSERT INTO recipe_images
         (id, recipe_id, media_type, image_bytes, width, height, sha256, alt_text,
          source_url, source_license, source_attribution, version, updated_at)
         VALUES (?, ?, 'image/jpeg', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(recipe_id) DO UPDATE SET
           id = excluded.id, media_type = excluded.media_type, image_bytes = excluded.image_bytes,
           width = excluded.width, height = excluded.height, sha256 = excluded.sha256,
           alt_text = excluded.alt_text, source_url = excluded.source_url,
           source_license = excluded.source_license,
           source_attribution = excluded.source_attribution, version = excluded.version,
           updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(recipe_id)
    .bind(&image.bytes)
    .bind(i64::from(image.width))
    .bind(i64::from(image.height))
    .bind(image.sha256.as_slice())
    .bind(&image.metadata.alt_text)
    .bind(&image.metadata.source_url)
    .bind(&image.metadata.source_license)
    .bind(&image.metadata.source_attribution)
    .bind(version)
    .bind(Utc::now().timestamp())
    .execute(&mut **transaction)
    .await?;
    Ok(id)
}

pub async fn delete_image(
    transaction: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM recipe_images WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub(crate) async fn session_upload_image(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Path(recipe_id): Path<String>,
    multipart: Multipart,
) -> AppResult<axum::Json<RecipeVersion>> {
    let idempotency_key = cover_idempotency_key(&headers)?;
    let expected_version = expected_recipe_version(&headers)?;
    let image = parse_image_multipart(multipart).await?;
    let fingerprint = cover_mutation_fingerprint(
        "upload",
        &[
            image.sha256.as_slice(),
            image.metadata.alt_text.as_bytes(),
            image
                .metadata
                .source_url
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
            image
                .metadata
                .source_license
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
            image
                .metadata
                .source_attribution
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        ],
    );
    let mut transaction = state.pool.begin().await?;
    if let Some(version) = replay_cover_mutation(
        &mut transaction,
        &user.id,
        &recipe_id,
        &idempotency_key,
        "upload",
        &fingerprint,
    )
    .await?
    {
        transaction.commit().await?;
        return Ok(axum::Json(RecipeVersion { version }));
    }
    let version =
        claim_owned_recipe_version(&mut transaction, &recipe_id, &user.id, expected_version)
            .await?;
    upsert_image(&mut transaction, &recipe_id, &image).await?;
    record_cover_mutation(
        &mut transaction,
        &user.id,
        &recipe_id,
        &idempotency_key,
        "upload",
        &fingerprint,
        version,
    )
    .await?;
    transaction.commit().await?;
    Ok(axum::Json(RecipeVersion { version }))
}

pub(crate) async fn session_delete_image(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Path(recipe_id): Path<String>,
) -> AppResult<axum::Json<RecipeVersion>> {
    let idempotency_key = cover_idempotency_key(&headers)?;
    let expected_version = expected_recipe_version(&headers)?;
    let fingerprint = cover_mutation_fingerprint("delete", &[]);
    let mut transaction = state.pool.begin().await?;
    if let Some(version) = replay_cover_mutation(
        &mut transaction,
        &user.id,
        &recipe_id,
        &idempotency_key,
        "delete",
        &fingerprint,
    )
    .await?
    {
        transaction.commit().await?;
        return Ok(axum::Json(RecipeVersion { version }));
    }
    let version =
        claim_owned_recipe_version(&mut transaction, &recipe_id, &user.id, expected_version)
            .await?;
    delete_image(&mut transaction, &recipe_id).await?;
    record_cover_mutation(
        &mut transaction,
        &user.id,
        &recipe_id,
        &idempotency_key,
        "delete",
        &fingerprint,
        version,
    )
    .await?;
    transaction.commit().await?;
    Ok(axum::Json(RecipeVersion { version }))
}

pub(crate) async fn session_update_image_metadata(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Path(recipe_id): Path<String>,
    axum::Json(input): axum::Json<ImageMetadataUpdate>,
) -> AppResult<axum::Json<RecipeVersion>> {
    let idempotency_key = cover_idempotency_key(&headers)?;
    let expected_version = expected_recipe_version(&headers)?;
    let metadata = validate_metadata(
        &input.alt_text,
        Some(&input.source_url),
        Some(&input.source_license),
        Some(&input.source_attribution),
    )?;
    let fingerprint = cover_mutation_fingerprint(
        "metadata",
        &[
            metadata.alt_text.as_bytes(),
            metadata
                .source_url
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
            metadata
                .source_license
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
            metadata
                .source_attribution
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        ],
    );
    let mut transaction = state.pool.begin().await?;
    if let Some(version) = replay_cover_mutation(
        &mut transaction,
        &user.id,
        &recipe_id,
        &idempotency_key,
        "metadata",
        &fingerprint,
    )
    .await?
    {
        transaction.commit().await?;
        return Ok(axum::Json(RecipeVersion { version }));
    }
    let version =
        claim_owned_recipe_version(&mut transaction, &recipe_id, &user.id, expected_version)
            .await?;
    let updated = sqlx::query(
        "UPDATE recipe_images SET alt_text = ?, source_url = ?, source_license = ?,
           source_attribution = ?, version = version + 1, updated_at = ?
         WHERE recipe_id = ?",
    )
    .bind(metadata.alt_text)
    .bind(metadata.source_url)
    .bind(metadata.source_license)
    .bind(metadata.source_attribution)
    .bind(Utc::now().timestamp())
    .bind(&recipe_id)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    record_cover_mutation(
        &mut transaction,
        &user.id,
        &recipe_id,
        &idempotency_key,
        "metadata",
        &fingerprint,
        version,
    )
    .await?;
    transaction.commit().await?;
    Ok(axum::Json(RecipeVersion { version }))
}

fn cover_idempotency_key(headers: &HeaderMap) -> AppResult<String> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::Validation("Idempotency-Key header is required".into()))?;
    if !(8..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AppError::Validation(
            "Idempotency-Key must contain 8 to 128 URL-safe characters".into(),
        ));
    }
    Ok(key.to_string())
}

fn cover_mutation_fingerprint(operation: &str, fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((operation.len() as u64).to_be_bytes());
    digest.update(operation.as_bytes());
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

async fn replay_cover_mutation(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    recipe_id: &str,
    idempotency_key: &str,
    operation: &str,
    fingerprint: &[u8; 32],
) -> AppResult<Option<i64>> {
    let prior = sqlx::query(
        "SELECT operation, request_fingerprint, result_version
         FROM recipe_cover_mutations
         WHERE owner_id = ? AND recipe_id = ? AND idempotency_key = ?",
    )
    .bind(owner_id)
    .bind(recipe_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(prior) = prior else {
        return Ok(None);
    };
    let prior_operation: String = prior.get("operation");
    let prior_fingerprint: Vec<u8> = prior.get("request_fingerprint");
    if prior_operation != operation || prior_fingerprint.as_slice() != fingerprint {
        return Err(AppError::Conflict(
            "Idempotency-Key was already used for a different cover mutation".into(),
        ));
    }
    Ok(Some(prior.get("result_version")))
}

async fn record_cover_mutation(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    recipe_id: &str,
    idempotency_key: &str,
    operation: &str,
    fingerprint: &[u8; 32],
    result_version: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO recipe_cover_mutations
         (owner_id, recipe_id, idempotency_key, operation, request_fingerprint,
          result_version, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(owner_id)
    .bind(recipe_id)
    .bind(idempotency_key)
    .bind(operation)
    .bind(fingerprint.as_slice())
    .bind(result_version)
    .bind(Utc::now().timestamp())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn claim_owned_recipe_version(
    transaction: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
    owner_id: &str,
    expected_version: i64,
) -> AppResult<i64> {
    let version: Option<i64> = sqlx::query_scalar(
        "UPDATE recipes SET version = version + 1, updated_at = CURRENT_TIMESTAMP
         WHERE id = ? AND owner_id = ? AND version = ?
         RETURNING version",
    )
    .bind(recipe_id)
    .bind(owner_id)
    .bind(expected_version)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(version) = version {
        return Ok(version);
    }
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM recipes WHERE id = ? AND owner_id = ?")
            .bind(recipe_id)
            .bind(owner_id)
            .fetch_optional(&mut **transaction)
            .await?;
    if exists.is_none() {
        Err(AppError::NotFound)
    } else {
        Err(AppError::Conflict(
            "recipe changed since it was opened; reload the latest version and compare your edits"
                .into(),
        ))
    }
}

pub async fn get_recipe_image(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(image_id): Path<String>,
) -> AppResult<Response<Body>> {
    let row = sqlx::query(
        "SELECT i.image_bytes, i.media_type, i.sha256, i.version, r.visibility, r.owner_id
         FROM recipe_images i JOIN recipes r ON r.id = i.recipe_id WHERE i.id = ?",
    )
    .bind(&image_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let visibility: String = row.get("visibility");
    let owner_id: String = row.get("owner_id");
    let public = visibility == "public";
    if !public {
        let session_owner = if let Some(cookie) = jar.get(state.auth.session_cookie_name()) {
            state
                .auth
                .sessions
                .authenticate(cookie.value(), Utc::now())
                .await
                .map_err(|_| AppError::AuthUnavailable)?
                .map(|context| context.user_id.as_str().to_string())
        } else {
            None
        };
        let token_principal = if session_owner.is_none() && headers.contains_key(AUTHORIZATION) {
            Some(authenticate_token(&state.pool, &headers, SCOPE_RECIPES_READ).await?)
        } else {
            None
        };
        if session_owner.as_deref() != Some(&owner_id)
            && token_principal
                .as_ref()
                .map(|principal| principal.owner_id.as_str())
                != Some(&owner_id)
        {
            return Err(AppError::NotFound);
        }
        if let Some(principal) = token_principal {
            audit_private_image_read(&state, &principal, &image_id).await?;
        }
    }
    let sha: Vec<u8> = row.get("sha256");
    let version: i64 = row.get("version");
    let etag = format!("\"{}-v{version}\"", bytes_hex(&sha));
    if headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str())
    {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .map_err(|_| AppError::Data("invalid image response".into()));
    }
    let bytes: Vec<u8> = row.get("image_bytes");
    let cache = if public {
        "public, max-age=0, must-revalidate"
    } else {
        "private, no-store"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, row.get::<String, _>("media_type"))
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .header(ETAG, etag)
        .header(CACHE_CONTROL, cache)
        .header(
            "x-content-type-options",
            HeaderValue::from_static("nosniff"),
        )
        .body(Body::from(bytes))
        .map_err(|_| AppError::Data("invalid image response".into()))
}

async fn audit_private_image_read(
    state: &AppState,
    principal: &TokenPrincipal,
    image_id: &str,
) -> AppResult<()> {
    let recipe_id: String = sqlx::query_scalar("SELECT recipe_id FROM recipe_images WHERE id = ?")
        .bind(image_id)
        .fetch_one(&state.pool)
        .await?;
    let mut transaction = state.pool.begin().await?;
    record_token_audit(
        &mut transaction,
        principal,
        "recipe.image.read",
        Some(&recipe_id),
        Utc::now().timestamp(),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

fn bounded_text(field: &str, value: &str, max: usize, allow_empty: bool) -> AppResult<String> {
    let value = value.trim();
    if !allow_empty && value.is_empty() {
        return Err(AppError::Validation(format!("{field} is required")));
    }
    if value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(AppError::Validation(format!(
            "{field} must contain at most {max} visible characters"
        )));
    }
    Ok(value.to_string())
}

fn optional_bounded_text(
    field: &str,
    value: Option<&str>,
    max: usize,
) -> AppResult<Option<String>> {
    value
        .map(|value| bounded_text(field, value, max, true))
        .transpose()
        .map(|value| value.filter(|value| !value.is_empty()))
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|value| format!("{value:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use image::{ImageEncoder, RgbaImage};

    fn metadata() -> ImageMetadataInput {
        ImageMetadataInput {
            alt_text: "A test image".into(),
            source_url: None,
            source_license: None,
            source_attribution: None,
        }
    }

    #[test]
    fn image_pipeline_accepts_png_and_strips_alpha_metadata_by_reencoding_jpeg() {
        let source = RgbaImage::from_pixel(32, 24, image::Rgba([10, 20, 30, 128]));
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&source, 32, 24, image::ExtendedColorType::Rgba8)
            .unwrap();
        let output = process_image(bytes, metadata()).unwrap();
        assert_eq!(
            image::guess_format(&output.bytes).unwrap(),
            ImageFormat::Jpeg
        );
        assert_eq!((output.width, output.height), (32, 24));
        assert!(output.bytes.len() <= MAX_STORED_BYTES);
    }

    #[test]
    fn image_pipeline_rejects_truncated_oversized_and_polyglot_like_inputs() {
        assert!(process_image(vec![0xff, 0xd8, 0xff], metadata()).is_err());
        assert!(
            process_image(b"<svg><script>alert(1)</script></svg>".to_vec(), metadata()).is_err()
        );
        let mut oversized = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
        oversized.extend_from_slice(b"not-a-real-png-with-javascript");
        assert!(process_image(oversized, metadata()).is_err());
    }

    #[test]
    fn image_pipeline_accepts_jpeg_png_and_webp_magic_bytes() {
        let source = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 6, Rgb([12, 34, 56])));
        for format in [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::WebP] {
            let mut bytes = Cursor::new(Vec::new());
            source.write_to(&mut bytes, format).unwrap();
            let output = process_image(bytes.into_inner(), metadata()).unwrap();
            assert_eq!(
                image::guess_format(&output.bytes).unwrap(),
                ImageFormat::Jpeg
            );
            assert_eq!((output.width, output.height), (8, 6));
        }
    }
}

use axum::{
    extract::{MatchedPath, Path, Query, State},
    http::{header::SET_COOKIE, HeaderMap, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use axum_extra::extract::cookie::CookieJar;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{NaiveDate, Utc};
use rust_web_auth_core::{AuthenticatedContext, LoginCallback, LoginError, UserId};
use rust_web_auth_email_otp::{EmailOtpRequest, EmailOtpVerification};
use rust_web_auth_http::{expired_cookie, issued_cookie};
use rust_web_auth_passkey::{
    KnownUserRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool, Transaction};
use url::Url;
use uuid::Uuid;

use crate::{
    account_assistance, ai_recipes,
    auth::{load_user, upsert_development_user, GoogleCompletionError},
    config::AuthMode,
    error::{AppError, AppResult},
    images, imports,
    ingredients::{
        clean_display_name, match_reason, normalize_ingredient, split_ingredient_list, MatchReason,
    },
    models::{
        normalize_nickname, IngredientAlternativeInput, IngredientGroupInput, IngredientInput,
        LocaleInput, PantryItem, PantryItemInput, PantryStatus, ProfileInput, Recipe, RecipeImage,
        RecipeInput, RecipeSource, RecommendationRequest, RecommendationSettings, User,
    },
    moderation,
    passwordless::{
        email_address, map_email_error, map_passkey_error, normalize_email, purpose,
        PasskeySummary, EMAIL_LINK_PURPOSE, EMAIL_SIGN_IN_PURPOSE,
    },
    recommendation::{recommend_limited_with_shopping, Recommendation},
    shopping, AppState,
};

const MAX_PANTRY_ITEMS_PER_OWNER: i64 = 500;
const MAX_INGREDIENT_ALIASES_PER_OWNER: i64 = 2_000;
const MAX_INGREDIENT_SUGGESTION_CANDIDATES: i64 = 5_000;
const MAX_RECOMMENDATION_CANDIDATES: i64 = 10_000;
const MAX_RECOMMENDATION_INGREDIENT_ROWS: i64 = MAX_RECOMMENDATION_CANDIDATES * 100;

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/me", get(me))
        .route("/api/settings/profile", put(update_profile))
        .route("/api/settings/locale", put(update_locale))
        .route("/api/auth/logout-all", post(logout_all))
        .route("/api/settings/security", get(security_settings))
        .route(
            "/api/settings/security/email/request",
            post(link_email_request),
        )
        .route(
            "/api/settings/security/email/verify",
            post(link_email_verify),
        )
        .route(
            "/api/settings/security/passkeys/register/start",
            post(passkey_registration_start),
        )
        .route(
            "/api/settings/security/passkeys/register/finish",
            post(passkey_registration_finish),
        )
        .route(
            "/api/settings/security/passkeys/{id}",
            axum::routing::delete(revoke_passkey),
        )
        .route("/api/recipes", get(list_recipes).post(create_recipe))
        .route(
            "/api/recipes/{id}",
            get(get_recipe).put(update_recipe).delete(delete_recipe),
        )
        .route(
            "/api/recipes/{id}/favorite",
            put(favorite_recipe).delete(unfavorite_recipe),
        )
        .route("/api/favorite-accounts", get(list_favorite_accounts))
        .route(
            "/api/authors/{handle}/favorite",
            put(favorite_account).delete(unfavorite_account),
        )
        .route(
            "/api/recipes/{id}/cover",
            post(images::session_upload_image)
                .put(images::session_update_image_metadata)
                .delete(images::session_delete_image)
                .layer(images::upload_body_limit()),
        )
        .route("/api/recipe-revisions", get(ai_recipes::list_revisions))
        .route(
            "/api/recipe-revisions/{id}/accept",
            post(ai_recipes::accept_revision),
        )
        .route(
            "/api/recipe-revisions/{id}",
            axum::routing::delete(ai_recipes::reject_revision),
        )
        .route(
            "/api/recipe-revisions/{id}/cover",
            get(ai_recipes::get_revision_image),
        )
        .route("/api/pantry", get(list_pantry).put(upsert_pantry))
        .route(
            "/api/shopping-list",
            get(shopping::list_shopping_items).post(shopping::create_manual_item),
        )
        .route(
            "/api/shopping-list/{id}",
            put(shopping::update_manual_item).delete(shopping::delete_item),
        )
        .route(
            "/api/shopping-list/{id}/purchase",
            post(shopping::purchase_item),
        )
        .route(
            "/api/shopping-list/purchases/{id}/undo",
            post(shopping::undo_purchase),
        )
        .route(
            "/api/recipes/{id}/shopping-list-items",
            post(shopping::add_recipe_ingredient),
        )
        .route("/api/ingredients/suggestions", get(ingredient_suggestions))
        .route(
            "/api/settings/recommendations",
            get(get_settings).put(update_settings),
        )
        .route("/api/recommendations", post(get_recommendations))
        .route(
            "/api/settings/import-tokens",
            get(imports::list_import_tokens).post(imports::create_import_token),
        )
        .route(
            "/api/settings/import-tokens/{id}",
            axum::routing::delete(imports::revoke_import_token),
        )
        .route("/api/import-drafts", get(imports::list_import_drafts))
        .route(
            "/api/capability-claims/content-moderator/passkey/start",
            post(moderation::claim_start),
        )
        .route(
            "/api/capability-claims/content-moderator/passkey/finish",
            post(moderation::claim_finish),
        )
        .route(
            "/api/capability-claims/account-assistant/passkey/start",
            post(account_assistance::claim_start),
        )
        .route(
            "/api/capability-claims/account-assistant/passkey/finish",
            post(account_assistance::claim_finish),
        )
        .route("/api/recipes/{id}/reports", post(moderation::report_recipe))
        .route("/api/notifications", get(moderation::list_notifications))
        .route(
            "/api/notifications/{id}/read",
            put(moderation::read_notification),
        )
        .route(
            "/api/recipes/{id}/appeals",
            get(moderation::list_appeals).post(moderation::create_appeal),
        )
        .route(
            "/api/admin/reauth/passkey/start",
            post(moderation::reauth_start),
        )
        .route(
            "/api/admin/reauth/passkey/finish",
            post(moderation::reauth_finish),
        )
        .route(
            "/api/admin/account-assistance/inspect",
            post(account_assistance::inspect),
        )
        .route(
            "/api/admin/account-assistance/otp",
            post(account_assistance::issue_otp),
        )
        .route("/api/admin/moderation/queue", get(moderation::queue))
        .route(
            "/api/admin/moderation/cases/{id}",
            get(moderation::case_detail),
        )
        .route(
            "/api/admin/moderation/cases/{id}/evidence-image",
            get(moderation::case_evidence_image),
        )
        .route(
            "/api/admin/moderation/recipes/{id}/review",
            post(moderation::mark_reviewed),
        )
        .route(
            "/api/admin/moderation/recipes/{id}/restrict",
            post(moderation::restrict),
        )
        .route(
            "/api/admin/moderation/recipes/{id}/reinstate",
            post(moderation::reinstate),
        )
        .route(
            "/api/admin/moderation/appeals/{id}/resolve",
            post(moderation::resolve_appeal),
        )
        .route("/api/admin/audit-events", get(moderation::audit_events))
        .route(
            "/api/admin/audit-events/{id}",
            get(moderation::audit_detail),
        )
        .route(
            "/api/admin/audit-events/export",
            get(moderation::audit_export),
        )
        .route(
            "/api/import-drafts/{id}",
            get(imports::get_import_draft)
                .put(imports::update_import_draft)
                .delete(imports::discard_import_draft),
        )
        .route(
            "/api/import-drafts/{id}/accept",
            post(imports::accept_import_draft),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authenticate_request,
        ));
    Router::new()
        .route("/health", get(health))
        .route("/api/auth/config", get(auth_config))
        .route("/api/auth/dev-login", post(dev_login))
        .route("/api/auth/google/start", get(google_start))
        .route("/api/auth/google/callback", get(google_callback))
        .route("/api/auth/email/request", post(email_sign_in_request))
        .route("/api/auth/email/verify", post(email_sign_in_verify))
        .route(
            "/api/auth/passkey/start",
            post(passkey_authentication_start),
        )
        .route(
            "/api/auth/passkey/finish",
            post(passkey_authentication_finish),
        )
        .route("/api/auth/logout", post(logout))
        .route("/api/authors/{handle}", get(public_author_profile))
        .route(
            "/api/ai/capabilities",
            get(imports::inspect_ai_capabilities),
        )
        .route("/api/import/drafts", post(imports::create_import_draft))
        .route(
            "/api/ai/recipes",
            get(ai_recipes::list_ai_recipes).post(ai_recipes::create_ai_recipe),
        )
        .route(
            "/api/ai/recipes/{id}",
            get(ai_recipes::get_ai_recipe).put(ai_recipes::update_ai_recipe),
        )
        .route(
            "/api/ai/recipes/{id}/cover",
            post(ai_recipes::upload_ai_image)
                .delete(ai_recipes::delete_ai_image)
                .layer(images::upload_body_limit()),
        )
        .route("/api/recipe-images/{id}", get(images::get_recipe_image))
        .merge(protected)
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorPageQuery {
    cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicAuthorProfile {
    handle: String,
    nickname: Option<String>,
    recipes: Vec<Recipe>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FavoriteAuthor {
    handle: String,
    nickname: Option<String>,
}

async fn public_author_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(handle): Path<String>,
    Query(query): Query<AuthorPageQuery>,
) -> AppResult<Json<PublicAuthorProfile>> {
    enforce_public_api_rate_limit(
        &state.pool,
        &format!(
            "author-profile:{}",
            rate_limit_key(&headers, &state.config.oauth_state_digest_key)
        ),
        120,
    )
    .await?;
    let author: (String, Option<String>) =
        sqlx::query_as("SELECT id, public_nickname FROM users WHERE public_handle = ?")
            .bind(&handle)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;
    let cursor = query
        .cursor
        .as_deref()
        .map(decode_author_cursor)
        .transpose()?;
    let rows: Vec<(String, String)> = if let Some((created_at, recipe_id)) = cursor {
        sqlx::query_as(
            "SELECT id, created_at FROM recipes
             WHERE owner_id = ? AND id IN (SELECT id FROM public_eligible_recipes)
               AND (created_at < ? OR (created_at = ? AND id < ?))
             ORDER BY created_at DESC, id DESC LIMIT 21",
        )
        .bind(&author.0)
        .bind(&created_at)
        .bind(&created_at)
        .bind(&recipe_id)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT id, created_at FROM recipes
             WHERE owner_id = ? AND id IN (SELECT id FROM public_eligible_recipes)
             ORDER BY created_at DESC, id DESC LIMIT 21",
        )
        .bind(&author.0)
        .fetch_all(&state.pool)
        .await?
    };
    let has_more = rows.len() > 20;
    let page_rows = rows.into_iter().take(20).collect::<Vec<_>>();
    let next_cursor = has_more
        .then(|| {
            page_rows
                .last()
                .map(|(id, created)| encode_author_cursor(created, id))
        })
        .flatten();
    let mut recipes = Vec::with_capacity(page_rows.len());
    for (recipe_id, _) in page_rows {
        let recipe = load_visible_recipes(&state.pool, "", Some(&recipe_id))
            .await?
            .into_iter()
            .next()
            .ok_or(AppError::NotFound)?;
        recipes.push(recipe);
    }
    Ok(Json(PublicAuthorProfile {
        handle,
        nickname: author.1,
        recipes,
        next_cursor,
    }))
}

async fn list_favorite_accounts(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Vec<FavoriteAuthor>>> {
    let rows = sqlx::query_as(
        "SELECT u.public_handle, u.public_nickname
         FROM favorite_accounts fa
         JOIN users u ON u.id = fa.author_id
         WHERE fa.viewer_id = ?
         ORDER BY lower(COALESCE(u.public_nickname, '')), u.public_handle",
    )
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(handle, nickname)| FavoriteAuthor { handle, nickname })
            .collect(),
    ))
}

async fn favorite_account(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(handle): Path<String>,
) -> AppResult<StatusCode> {
    enforce_public_api_rate_limit(&state.pool, &format!("favorite-account:{}", user.id), 30)
        .await?;
    let author_id: String = sqlx::query_scalar("SELECT id FROM users WHERE public_handle = ?")
        .bind(handle)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    if author_id == user.id {
        return Err(AppError::Validation(
            "you cannot favorite your own account".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO favorite_accounts (viewer_id, author_id, created_at)
         VALUES (?, ?, ?) ON CONFLICT(viewer_id, author_id) DO NOTHING",
    )
    .bind(user.id)
    .bind(author_id)
    .bind(Utc::now().timestamp())
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unfavorite_account(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(handle): Path<String>,
) -> AppResult<StatusCode> {
    enforce_public_api_rate_limit(&state.pool, &format!("favorite-account:{}", user.id), 30)
        .await?;
    let author_id: String = sqlx::query_scalar("SELECT id FROM users WHERE public_handle = ?")
        .bind(handle)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    sqlx::query("DELETE FROM favorite_accounts WHERE viewer_id = ? AND author_id = ?")
        .bind(user.id)
        .bind(author_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn enforce_public_api_rate_limit(
    pool: &SqlitePool,
    rate_key: &str,
    limit: i64,
) -> AppResult<()> {
    let now = Utc::now().timestamp();
    let window_started_at = now - now.rem_euclid(60);
    let request_count: i64 = sqlx::query_scalar(
        "INSERT INTO public_api_rate_limits (rate_key, window_started_at, request_count)
         VALUES (?, ?, 1)
         ON CONFLICT(rate_key) DO UPDATE SET
           window_started_at = CASE
             WHEN public_api_rate_limits.window_started_at < excluded.window_started_at
             THEN excluded.window_started_at ELSE public_api_rate_limits.window_started_at END,
           request_count = CASE
             WHEN public_api_rate_limits.window_started_at < excluded.window_started_at
             THEN 1 ELSE public_api_rate_limits.request_count + 1 END
         RETURNING request_count",
    )
    .bind(rate_key)
    .bind(window_started_at)
    .fetch_one(pool)
    .await?;
    if request_count > limit {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

fn encode_author_cursor(created_at: &str, recipe_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("{created_at}\n{recipe_id}"))
}

fn decode_author_cursor(cursor: &str) -> AppResult<(String, String)> {
    let value = URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| AppError::Validation("invalid author page cursor".into()))?;
    let (created_at, recipe_id) = value
        .split_once('\n')
        .ok_or_else(|| AppError::Validation("invalid author page cursor".into()))?;
    if created_at.is_empty() || recipe_id.is_empty() {
        return Err(AppError::Validation("invalid author page cursor".into()));
    }
    Ok((created_at.to_string(), recipe_id.to_string()))
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevLogin {
    display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthConfig {
    mode: &'static str,
    google: bool,
    email_otp: bool,
    passkey: bool,
}

#[derive(Deserialize)]
struct GoogleCallback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmailOtpRequestInput {
    email: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmailOtpVerifyInput {
    challenge_id: String,
    code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EmailOtpChallengeResponse {
    challenge_id: String,
    expires_at: String,
    resend_allowed_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecuritySettings {
    linked_emails: Vec<String>,
    google_email: Option<String>,
    google_sign_in_status: GoogleSignInStatus,
    passkeys: Vec<PasskeySummary>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum GoogleSignInStatus {
    Linked,
    Eligible,
    Unsupported,
    Unavailable,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PasskeyRegistrationInput {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PasskeyAuthenticationInput {
    email: String,
}

#[derive(Deserialize)]
struct IngredientSuggestionQuery {
    #[serde(default)]
    q: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IngredientSuggestion {
    canonical_ingredient_id: String,
    display_name: String,
    matched_text: String,
    reason: MatchReason,
}

#[derive(Clone)]
pub(crate) struct CurrentUser(pub User);

#[derive(Clone)]
pub(crate) struct CurrentAuth(pub AuthenticatedContext);

async fn auth_config(State(state): State<AppState>) -> Json<AuthConfig> {
    Json(AuthConfig {
        mode: if state.auth.mode == AuthMode::Google {
            "production"
        } else {
            "development"
        },
        google: state.auth.login.is_some(),
        email_otp: true,
        passkey: true,
    })
}

async fn dev_login(
    State(state): State<AppState>,
    Json(input): Json<DevLogin>,
) -> AppResult<impl IntoResponse> {
    if state.auth.mode != AuthMode::Development {
        return Err(AppError::NotFound);
    }
    let user = upsert_development_user(&state.pool, &input.display_name).await?;
    issue_user_session(
        &state,
        UserId::new(user.id).map_err(|_| AppError::Unauthorized)?,
    )
    .await
}

async fn email_sign_in_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<EmailOtpRequestInput>,
) -> AppResult<Json<EmailOtpChallengeResponse>> {
    request_email_otp(&state, &headers, &input.email, EMAIL_SIGN_IN_PURPOSE, "/").await
}

async fn email_sign_in_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<EmailOtpVerifyInput>,
) -> AppResult<impl IntoResponse> {
    if let Some(response) =
        account_assistance::try_verify(&state, &headers, &input.challenge_id, &input.code).await?
    {
        return Ok(response.into_response());
    }
    let verified = state
        .auth
        .passwordless
        .email_otp
        .verify(
            EmailOtpVerification {
                challenge_id: &input.challenge_id,
                code: &input.code,
                ip_partition: &rate_limit_key(&headers, &state.config.email_otp_digest_key),
            },
            Utc::now(),
        )
        .await
        .map_err(map_email_error)?;
    if verified.purpose.as_str() != EMAIL_SIGN_IN_PURPOSE {
        return Err(AppError::Unauthorized);
    }
    let normalized = normalize_email(verified.email.as_str()).map_err(map_email_error)?;
    let user_id = state
        .auth
        .passwordless
        .store
        .resolve_email_sign_in(&normalized)
        .await?;
    Ok(issue_user_session(&state, user_id).await?.into_response())
}

async fn link_email_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentAuth(auth)): Extension<CurrentAuth>,
    Json(input): Json<EmailOtpRequestInput>,
) -> AppResult<Json<EmailOtpChallengeResponse>> {
    account_assistance::ensure_not_assisted_session(&state, &auth).await?;
    let normalized = normalize_email(&input.email).map_err(map_email_error)?;
    let google_emails = state
        .auth
        .passwordless
        .store
        .google_verified_emails(&user.id)
        .await?;
    if !google_emails.is_empty() && !google_emails.iter().any(|value| value == &normalized) {
        return Err(AppError::Validation(
            "email must match the verified email on the linked Google identity".into(),
        ));
    }
    let challenge = request_email_otp(
        &state,
        &headers,
        &normalized,
        EMAIL_LINK_PURPOSE,
        "/settings",
    )
    .await?;
    let expires_at = chrono::DateTime::parse_from_rfc3339(&challenge.expires_at)
        .map_err(|_| AppError::AuthUnavailable)?
        .with_timezone(&Utc);
    state
        .auth
        .passwordless
        .store
        .bind_link_target(&challenge.challenge_id, &user.id, expires_at)
        .await?;
    Ok(challenge)
}

async fn link_email_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentAuth(auth)): Extension<CurrentAuth>,
    Json(input): Json<EmailOtpVerifyInput>,
) -> AppResult<Json<SecuritySettings>> {
    account_assistance::ensure_not_assisted_session(&state, &auth).await?;
    let now = Utc::now();
    let verified = state
        .auth
        .passwordless
        .email_otp
        .verify(
            EmailOtpVerification {
                challenge_id: &input.challenge_id,
                code: &input.code,
                ip_partition: &rate_limit_key(&headers, &state.config.email_otp_digest_key),
            },
            now,
        )
        .await
        .map_err(map_email_error)?;
    if verified.purpose.as_str() != EMAIL_LINK_PURPOSE {
        return Err(AppError::Unauthorized);
    }
    state
        .auth
        .passwordless
        .store
        .consume_link_target(&input.challenge_id, &user.id, now)
        .await?;
    let normalized = normalize_email(verified.email.as_str()).map_err(map_email_error)?;
    state
        .auth
        .passwordless
        .store
        .link_email(&user.id, &normalized)
        .await?;
    load_security_settings(&state, &user.id).await.map(Json)
}

async fn security_settings(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<SecuritySettings>> {
    load_security_settings(&state, &user.id).await.map(Json)
}

async fn passkey_registration_start(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(input): Json<PasskeyRegistrationInput>,
) -> AppResult<impl IntoResponse> {
    let name = validate_passkey_name(&input.name)?;
    let linked_emails = state
        .auth
        .passwordless
        .store
        .mailbox_verified_emails(&user.id)
        .await?;
    let user_name = linked_emails.first().ok_or_else(|| {
        AppError::Validation("link an email address before adding a passkey".into())
    })?;
    let user_id = UserId::new(user.id.clone()).map_err(|_| AppError::AuthUnavailable)?;
    let user_handle = state
        .auth
        .passwordless
        .store
        .ensure_user_handle(&user.id)
        .await?;
    let start = state
        .auth
        .passwordless
        .passkeys
        .start_registration(
            KnownUserRegistration {
                user_id,
                user_handle,
                user_name: user_name.clone(),
                user_display_name: user.display_name,
            },
            Utc::now(),
        )
        .await
        .map_err(map_passkey_error)?;
    state.auth.passwordless.remember_registration(
        start.ceremony_token.expose_for_cookie(),
        &user.id,
        name,
        Utc::now(),
    )?;
    let cookie = issued_cookie(
        &state.auth.passwordless.passkey_registration_cookie,
        start.ceremony_token.expose_for_cookie(),
        5 * 60,
    )
    .map_err(|_| AppError::AuthUnavailable)?;
    Ok(([(SET_COOKIE, cookie)], Json(start.challenge)))
}

async fn passkey_registration_finish(
    State(state): State<AppState>,
    jar: CookieJar,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(response): Json<RegisterPublicKeyCredential>,
) -> AppResult<impl IntoResponse> {
    let token = jar
        .get(state.auth.passwordless.passkey_registration_cookie.name())
        .map(|cookie| cookie.value())
        .ok_or(AppError::Unauthorized)?;
    let name = state
        .auth
        .passwordless
        .consume_registration(token, &user.id, Utc::now())?;
    state
        .auth
        .passwordless
        .store
        .set_pending_credential_name(&user.id, name)?;
    let result = state
        .auth
        .passwordless
        .passkeys
        .finish_registration(token, &response, Utc::now())
        .await;
    if result.is_err() {
        state
            .auth
            .passwordless
            .store
            .clear_pending_credential_name(&user.id);
    }
    result.map_err(map_passkey_error)?;
    let credential_row_id = state
        .auth
        .passwordless
        .store
        .take_registered_credential_id(&user.id)?;
    let credential = state
        .auth
        .passwordless
        .store
        .passkey_summary(&user.id, &credential_row_id)
        .await?;
    Ok((
        [(
            SET_COOKIE,
            expired_cookie(&state.auth.passwordless.passkey_registration_cookie)
                .map_err(|_| AppError::AuthUnavailable)?,
        )],
        (StatusCode::CREATED, Json(credential)),
    ))
}

async fn passkey_authentication_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasskeyAuthenticationInput>,
) -> AppResult<impl IntoResponse> {
    let normalized = normalize_email(&input.email).map_err(map_email_error)?;
    let ip_partition = rate_limit_key(&headers, &state.config.email_otp_digest_key);
    if !state
        .auth
        .passwordless
        .store
        .check_passkey_start(&normalized, &ip_partition, Utc::now())
        .await?
    {
        return Err(AppError::RateLimited);
    }
    let user_id = state
        .auth
        .passwordless
        .store
        .find_user_by_email(&normalized)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let start = state
        .auth
        .passwordless
        .passkeys
        .start_authentication(user_id, Utc::now())
        .await
        .map_err(map_passkey_error)?;
    let cookie = issued_cookie(
        &state.auth.passwordless.passkey_authentication_cookie,
        start.ceremony_token.expose_for_cookie(),
        5 * 60,
    )
    .map_err(|_| AppError::AuthUnavailable)?;
    Ok(([(SET_COOKIE, cookie)], Json(start.challenge)))
}

async fn passkey_authentication_finish(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(response): Json<PublicKeyCredential>,
) -> AppResult<impl IntoResponse> {
    let token = jar
        .get(state.auth.passwordless.passkey_authentication_cookie.name())
        .map(|cookie| cookie.value())
        .ok_or(AppError::Unauthorized)?;
    let user_id = state
        .auth
        .passwordless
        .passkeys
        .finish_authentication(token, &response, Utc::now())
        .await
        .map_err(map_passkey_error)?;
    let (mut headers, user) = issue_user_session_parts(&state, user_id).await?;
    headers.append(
        SET_COOKIE,
        expired_cookie(&state.auth.passwordless.passkey_authentication_cookie)
            .map_err(|_| AppError::AuthUnavailable)?,
    );
    Ok((headers, Json(user)))
}

async fn revoke_passkey(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentAuth(auth)): Extension<CurrentAuth>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    account_assistance::ensure_not_assisted_session(&state, &auth).await?;
    let user_id = UserId::new(user.id.clone()).map_err(|_| AppError::AuthUnavailable)?;
    let handle = state
        .auth
        .passwordless
        .store
        .ensure_user_handle(&user.id)
        .await?;
    let credential_id = state
        .auth
        .passwordless
        .store
        .credential_id_for_owner(&user.id, &id)
        .await?;
    state
        .auth
        .passwordless
        .passkeys
        .revoke_credential(&user_id, handle, &credential_id)
        .await
        .map_err(map_passkey_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn google_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let login = state.auth.login.as_ref().ok_or(AppError::NotFound)?;
    let start = login
        .begin(
            &rate_limit_key(&headers, &state.config.oauth_state_digest_key),
            Utc::now(),
        )
        .await
        .map_err(map_login_error)?;
    let cookie = issued_cookie(
        &state.auth.state_cookie,
        &start.state_cookie_value,
        seconds_until(start.expires_at),
    )
    .map_err(|_| AppError::AuthUnavailable)?;
    Ok((
        [(SET_COOKIE, cookie)],
        Redirect::temporary(start.authorization_url.as_str()),
    ))
}

async fn google_callback(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<GoogleCallback>,
) -> AppResult<impl IntoResponse> {
    let completion = match state
        .auth
        .complete_google(
            LoginCallback {
                state: query.state.as_deref(),
                state_cookie: jar
                    .get(state.auth.state_cookie_name())
                    .map(|cookie| cookie.value()),
                authorization_code: query.code.as_deref(),
                provider_reported_error: query.error.is_some(),
            },
            Utc::now(),
        )
        .await
    {
        Ok(completion) => completion,
        Err(GoogleCompletionError::UnsafeAccountLink) => {
            let mut headers = HeaderMap::new();
            headers.append(
                SET_COOKIE,
                expired_cookie(&state.auth.state_cookie).map_err(|_| AppError::AuthUnavailable)?,
            );
            return Ok((headers, Redirect::to("/?authError=google-account-conflict")));
        }
        Err(GoogleCompletionError::Login(error)) => return Err(map_login_error(error)),
    };
    let mut headers = HeaderMap::new();
    headers.append(
        SET_COOKIE,
        issued_cookie(
            &state.auth.session_cookie,
            &completion.cookie_value,
            seconds_until(completion.expires_at),
        )
        .map_err(|_| AppError::AuthUnavailable)?,
    );
    headers.append(
        SET_COOKIE,
        expired_cookie(&state.auth.state_cookie).map_err(|_| AppError::AuthUnavailable)?,
    );
    Ok((headers, Redirect::to("/")))
}

async fn logout(State(state): State<AppState>, jar: CookieJar) -> AppResult<impl IntoResponse> {
    if let Some(cookie) = jar.get(state.auth.session_cookie_name()) {
        state
            .auth
            .sessions
            .revoke_current(cookie.value(), Utc::now())
            .await
            .map_err(|_| AppError::AuthUnavailable)?;
    }
    let mut headers = HeaderMap::new();
    headers.append(
        SET_COOKIE,
        expired_cookie(&state.auth.session_cookie).map_err(|_| AppError::AuthUnavailable)?,
    );
    headers.append(
        SET_COOKIE,
        expired_cookie(&state.auth.state_cookie).map_err(|_| AppError::AuthUnavailable)?,
    );
    headers.append(
        SET_COOKIE,
        expired_cookie(&state.auth.passwordless.passkey_registration_cookie)
            .map_err(|_| AppError::AuthUnavailable)?,
    );
    headers.append(
        SET_COOKIE,
        expired_cookie(&state.auth.passwordless.passkey_authentication_cookie)
            .map_err(|_| AppError::AuthUnavailable)?,
    );
    Ok((headers, StatusCode::NO_CONTENT))
}

async fn logout_all(
    State(state): State<AppState>,
    Extension(CurrentAuth(context)): Extension<CurrentAuth>,
) -> AppResult<impl IntoResponse> {
    state
        .auth
        .sessions
        .revoke_all(&context, Utc::now())
        .await
        .map_err(|_| AppError::AuthUnavailable)?;
    Ok((
        [(
            SET_COOKIE,
            expired_cookie(&state.auth.session_cookie).map_err(|_| AppError::AuthUnavailable)?,
        )],
        StatusCode::NO_CONTENT,
    ))
}

async fn request_email_otp(
    state: &AppState,
    headers: &HeaderMap,
    email: &str,
    purpose_value: &str,
    continue_path: &str,
) -> AppResult<Json<EmailOtpChallengeResponse>> {
    let challenge = state
        .auth
        .passwordless
        .email_otp
        .request(
            EmailOtpRequest {
                email: email_address(email).map_err(map_email_error)?,
                purpose: purpose(purpose_value).map_err(map_email_error)?,
                continue_path: continue_path.to_string(),
                ip_partition: rate_limit_key(headers, &state.config.email_otp_digest_key),
            },
            Utc::now(),
        )
        .await
        .map_err(map_email_error)?;
    Ok(Json(EmailOtpChallengeResponse {
        challenge_id: challenge.challenge_id.expose_for_client().to_string(),
        expires_at: challenge.expires_at.to_rfc3339(),
        resend_allowed_at: challenge.resend_allowed_at.to_rfc3339(),
    }))
}

async fn issue_user_session(
    state: &AppState,
    user_id: UserId,
) -> AppResult<(HeaderMap, Json<User>)> {
    let (headers, user) = issue_user_session_parts(state, user_id).await?;
    Ok((headers, Json(user)))
}

async fn issue_user_session_parts(
    state: &AppState,
    user_id: UserId,
) -> AppResult<(HeaderMap, User)> {
    let completion = state
        .auth
        .sessions
        .issue(user_id.clone(), Utc::now())
        .await
        .map_err(|_| AppError::AuthUnavailable)?;
    let mut headers = HeaderMap::new();
    headers.append(
        SET_COOKIE,
        issued_cookie(
            &state.auth.session_cookie,
            &completion.cookie_value,
            seconds_until(completion.expires_at),
        )
        .map_err(|_| AppError::AuthUnavailable)?,
    );
    Ok((headers, load_user(&state.pool, &user_id).await?))
}

pub(crate) async fn issue_assisted_user_session(
    state: &AppState,
    user_id: UserId,
    assignment_id: &str,
    assignment_alias: &str,
    authenticated_at: i64,
    lifetime_seconds: i64,
) -> AppResult<(HeaderMap, Json<User>)> {
    let completion = state
        .auth
        .sessions
        .issue(user_id.clone(), Utc::now())
        .await
        .map_err(|_| AppError::AuthUnavailable)?;
    let expires_at = authenticated_at + lifetime_seconds;
    sqlx::query(
        "UPDATE auth_sessions
         SET auth_method = 'admin_assisted_otp', issuer_assignment_id = ?,
             issuer_assignment_alias = ?, authenticated_at = ?, expires_at = ?
         WHERE session_id = ? AND user_id = ?",
    )
    .bind(assignment_id)
    .bind(assignment_alias)
    .bind(authenticated_at)
    .bind(expires_at)
    .bind(completion.session_id.to_string())
    .bind(user_id.as_str())
    .execute(&state.pool)
    .await?;
    let mut headers = HeaderMap::new();
    headers.append(
        SET_COOKIE,
        issued_cookie(
            &state.auth.session_cookie,
            &completion.cookie_value,
            u64::try_from(lifetime_seconds).map_err(|_| AppError::AuthUnavailable)?,
        )
        .map_err(|_| AppError::AuthUnavailable)?,
    );
    let mut user = load_user(&state.pool, &user_id).await?;
    apply_assisted_user_projection(
        &mut user,
        crate::models::AssistedSession {
            issuer_alias: assignment_alias.to_string(),
            authenticated_at,
            expires_at,
        },
    );
    user.last_assisted_login_at = Some(authenticated_at);
    Ok((headers, Json(user)))
}

async fn load_security_settings(state: &AppState, user_id: &str) -> AppResult<SecuritySettings> {
    let linked_emails = state.auth.passwordless.store.linked_emails(user_id).await?;
    let google_emails = state
        .auth
        .passwordless
        .store
        .google_verified_emails(user_id)
        .await?;
    let google_sign_in_status = if !google_emails.is_empty() {
        GoogleSignInStatus::Linked
    } else if linked_emails.iter().any(|email| {
        email
            .rsplit_once('@')
            .is_some_and(|(_, domain)| domain == "gmail.com")
    }) {
        GoogleSignInStatus::Eligible
    } else if linked_emails.is_empty() {
        GoogleSignInStatus::Unavailable
    } else {
        GoogleSignInStatus::Unsupported
    };
    Ok(SecuritySettings {
        linked_emails,
        google_email: google_emails.into_iter().next(),
        google_sign_in_status,
        passkeys: state.auth.passwordless.store.list_passkeys(user_id).await?,
    })
}

fn validate_passkey_name(value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "passkey name must contain 1 to 80 visible characters".into(),
        ));
    }
    Ok(value.to_string())
}

async fn authenticate_request(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> AppResult<Response> {
    let cookie = jar
        .get(state.auth.session_cookie_name())
        .ok_or(AppError::Unauthorized)?;
    let context = state
        .auth
        .sessions
        .authenticate(cookie.value(), Utc::now())
        .await
        .map_err(|_| AppError::AuthUnavailable)?
        .ok_or(AppError::Unauthorized)?;
    let mut user = load_user(&state.pool, &context.user_id).await?;
    let session = sqlx::query(
        "SELECT auth_method, issuer_assignment_alias, authenticated_at, expires_at
         FROM auth_sessions WHERE session_id = ? AND user_id = ?",
    )
    .bind(context.session_id.to_string())
    .bind(context.user_id.as_str())
    .fetch_optional(&state.pool)
    .await?;
    if let Some(session) = session {
        if session.get::<String, _>("auth_method") == "admin_assisted_otp" {
            apply_assisted_user_projection(
                &mut user,
                crate::models::AssistedSession {
                    issuer_alias: session.get("issuer_assignment_alias"),
                    authenticated_at: session.get("authenticated_at"),
                    expires_at: session.get("expires_at"),
                },
            );
        }
    }
    if user.assisted_session.is_some()
        && !assisted_session_request_allowed(
            request.method(),
            request
                .extensions()
                .get::<MatchedPath>()
                .map(MatchedPath::as_str),
        )
    {
        return Err(AppError::Forbidden);
    }
    user.last_assisted_login_at = sqlx::query_scalar(
        "SELECT MAX(created_at) FROM account_assistance_notifications
         WHERE user_id = ? AND kind = 'account_assistance_consumed'",
    )
    .bind(context.user_id.as_str())
    .fetch_one(&state.pool)
    .await?;
    request.extensions_mut().insert(CurrentUser(user));
    request
        .extensions_mut()
        .insert(CurrentAuth(context.clone()));
    let mut response = next.run(request).await;
    if let Some(expires_at) = context.renewed_expires_at {
        response.headers_mut().append(
            SET_COOKIE,
            issued_cookie(
                &state.auth.session_cookie,
                cookie.value(),
                seconds_until(expires_at),
            )
            .map_err(|_| AppError::AuthUnavailable)?,
        );
    }
    Ok(response)
}

fn apply_assisted_user_projection(
    user: &mut User,
    assisted_session: crate::models::AssistedSession,
) {
    user.capabilities.clear();
    user.assisted_session = Some(assisted_session);
}

fn assisted_session_request_allowed(method: &Method, matched_path: Option<&str>) -> bool {
    if *method == Method::OPTIONS {
        return true;
    }
    if matches!(*method, Method::GET | Method::HEAD) {
        return matched_path.is_some_and(|path| {
            path != "/api/settings/security"
                && !path.starts_with("/api/settings/import-tokens")
                && !path.starts_with("/api/admin/")
        });
    }
    matches!(
        (method.as_str(), matched_path),
        ("PUT", Some("/api/settings/profile"))
            | ("PUT", Some("/api/settings/locale"))
            | ("POST", Some("/api/recipes"))
            | ("PUT", Some("/api/recipes/{id}"))
            | ("DELETE", Some("/api/recipes/{id}"))
            | ("PUT", Some("/api/recipes/{id}/favorite"))
            | ("DELETE", Some("/api/recipes/{id}/favorite"))
            | ("PUT", Some("/api/authors/{handle}/favorite"))
            | ("DELETE", Some("/api/authors/{handle}/favorite"))
            | ("POST", Some("/api/recipes/{id}/cover"))
            | ("PUT", Some("/api/recipes/{id}/cover"))
            | ("DELETE", Some("/api/recipes/{id}/cover"))
            | ("POST", Some("/api/recipe-revisions/{id}/accept"))
            | ("DELETE", Some("/api/recipe-revisions/{id}"))
            | ("PUT", Some("/api/pantry"))
            | ("POST", Some("/api/shopping-list"))
            | ("PUT", Some("/api/shopping-list/{id}"))
            | ("DELETE", Some("/api/shopping-list/{id}"))
            | ("POST", Some("/api/shopping-list/{id}/purchase"))
            | ("POST", Some("/api/shopping-list/purchases/{id}/undo"))
            | ("POST", Some("/api/recipes/{id}/shopping-list-items"))
            | ("PUT", Some("/api/settings/recommendations"))
            | ("POST", Some("/api/recommendations"))
            | ("POST", Some("/api/recipes/{id}/reports"))
            | ("PUT", Some("/api/notifications/{id}/read"))
            | ("POST", Some("/api/recipes/{id}/appeals"))
            | ("PUT", Some("/api/import-drafts/{id}"))
            | ("DELETE", Some("/api/import-drafts/{id}"))
            | ("POST", Some("/api/import-drafts/{id}/accept"))
    )
}

async fn me(Extension(CurrentUser(user)): Extension<CurrentUser>) -> Json<User> {
    Json(user)
}

async fn update_profile(
    State(state): State<AppState>,
    Extension(CurrentUser(mut user)): Extension<CurrentUser>,
    Json(input): Json<ProfileInput>,
) -> AppResult<Json<User>> {
    let nickname = normalize_nickname(input.nickname.as_deref())
        .map_err(|message| AppError::Validation(message.into()))?;
    sqlx::query(
        "UPDATE users SET public_nickname = ?, profile_onboarding_completed = 1 WHERE id = ?",
    )
    .bind(&nickname)
    .bind(&user.id)
    .execute(&state.pool)
    .await?;
    user.nickname = nickname;
    user.onboarding_completed = true;
    Ok(Json(user))
}

async fn update_locale(
    State(state): State<AppState>,
    Extension(CurrentUser(mut user)): Extension<CurrentUser>,
    Json(input): Json<LocaleInput>,
) -> AppResult<Json<User>> {
    if input.locale != "en" && input.locale != "zh-CN" {
        return Err(AppError::Validation(
            "locale must be en or zh-CN".to_string(),
        ));
    }
    sqlx::query("UPDATE users SET locale = ? WHERE id = ?")
        .bind(&input.locale)
        .bind(&user.id)
        .execute(&state.pool)
        .await?;
    user.locale = Some(input.locale);
    Ok(Json(user))
}

async fn list_recipes(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Vec<Recipe>>> {
    Ok(Json(
        load_visible_recipes(&state.pool, &user.id, None).await?,
    ))
}

async fn get_recipe(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Json<Recipe>> {
    load_visible_recipes(&state.pool, &user.id, Some(&id))
        .await?
        .into_iter()
        .next()
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn create_recipe(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(mut input): Json<RecipeInput>,
) -> AppResult<(StatusCode, Json<Recipe>)> {
    input.ingredients = expand_ingredient_inputs(input.ingredients);
    validate_recipe(&input)?;
    let id = Uuid::new_v4().to_string();
    save_recipe(&state.pool, &id, &user.id, &input, false).await?;
    let recipe = load_visible_recipes(&state.pool, &user.id, Some(&id))
        .await?
        .into_iter()
        .next()
        .ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(recipe)))
}

async fn update_recipe(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut input): Json<RecipeInput>,
) -> AppResult<Json<Recipe>> {
    let expected_version = expected_recipe_version(&headers)?;
    input.ingredients = expand_ingredient_inputs(input.ingredients);
    validate_recipe(&input)?;
    let mut transaction = state.pool.begin().await?;
    let previous: (String, String) = sqlx::query_as(
        "SELECT visibility, moderation_state FROM recipes WHERE id = ? AND owner_id = ?",
    )
    .bind(&id)
    .bind(&user.id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let previous_visibility = previous.0;
    let effective_visibility = if previous.1 == "restricted" {
        "private"
    } else {
        input.visibility.as_str()
    };
    if input.ingredient_groups.is_none() {
        let requires_groups: i64 = sqlx::query_scalar(
            "SELECT EXISTS(
               SELECT 1 FROM recipe_ingredient_groups g
               WHERE g.recipe_id = ? AND (
                 g.optional = 1 OR
                 g.note <> '' OR
                 (SELECT COUNT(*) FROM recipe_ingredient_alternatives a
                  WHERE a.group_id = g.id) > 1
               )
             )",
        )
        .bind(&id)
        .fetch_one(&mut *transaction)
        .await?;
        if requires_groups != 0 {
            return Err(AppError::Validation("ingredient_groups_required".into()));
        }
    }
    let result = sqlx::query(
        "UPDATE recipes SET title = ?, description = ?, instructions = ?, visibility = ?,
         publication_intent = ?, moderation_review_state = 'unreviewed',
         moderation_version = moderation_version + 1,
         difficulty = ?, estimated_calories = ?, source_url = ?, source_title = ?,
         source_author = ?, source_license = ?, source_attribution = ?,
         version = version + 1,
         updated_at = CURRENT_TIMESTAMP
         WHERE id = ? AND owner_id = ? AND version = ?",
    )
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(input.instructions.trim())
    .bind(effective_visibility)
    .bind(&input.visibility)
    .bind(input.difficulty)
    .bind(input.estimated_calories)
    .bind(optional_text(&input.source_url))
    .bind(optional_text(&input.source_title))
    .bind(optional_text(&input.source_author))
    .bind(optional_text(&input.source_license))
    .bind(optional_text(&input.source_attribution))
    .bind(&id)
    .bind(&user.id)
    .bind(expected_version)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::Conflict(
            "recipe changed since it was opened; reload the latest version and compare your edits"
                .into(),
        ));
    }
    let groups = effective_ingredient_groups(&input)?;
    replace_ingredient_groups(&mut transaction, &id, &user.id, &groups).await?;
    if previous_visibility == "public" && effective_visibility == "private" {
        sqlx::query(
            "UPDATE recipe_images SET id = ?, version = version + 1, updated_at = ?
             WHERE recipe_id = ?",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(Utc::now().timestamp())
        .bind(&id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    let recipe = load_visible_recipes(&state.pool, &user.id, Some(&id))
        .await?
        .into_iter()
        .next()
        .ok_or(AppError::NotFound)?;
    Ok(Json(recipe))
}

pub(crate) fn expected_recipe_version(headers: &HeaderMap) -> AppResult<i64> {
    headers
        .get("if-match")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("\"recipe-v"))
        .and_then(|value| value.strip_suffix('"'))
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            AppError::Validation("If-Match must use the form \"recipe-v<version>\"".into())
        })
}

async fn delete_recipe(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    moderation::delete_owned_recipe(&state, &user.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn favorite_recipe(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    ensure_visible_recipe(&state.pool, &user.id, &id).await?;
    sqlx::query("INSERT OR IGNORE INTO favorites (user_id, recipe_id) VALUES (?, ?)")
        .bind(user.id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unfavorite_recipe(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    sqlx::query("DELETE FROM favorites WHERE user_id = ? AND recipe_id = ?")
        .bind(user.id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ingredient_suggestions(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(query): Query<IngredientSuggestionQuery>,
) -> AppResult<Json<Vec<IngredientSuggestion>>> {
    let query_text = clean_display_name(&query.q);
    if query_text.is_empty() {
        return Ok(Json(Vec::new()));
    }
    if query_text.chars().count() > 120 {
        return Err(AppError::Validation(
            "ingredient search must be at most 120 characters".into(),
        ));
    }
    let candidates: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.id, c.display_name, c.normalized_name
         FROM ingredient_catalog c
         WHERE EXISTS (
             SELECT 1 FROM pantry_items p
             WHERE p.canonical_ingredient_id = c.id AND p.owner_id = ?
         ) OR EXISTS (
             SELECT 1
             FROM recipe_ingredients ri
             JOIN recipes r ON r.id = ri.recipe_id
             WHERE ri.canonical_ingredient_id = c.id
               AND (r.owner_id = ? OR r.id IN (SELECT id FROM public_eligible_recipes))
         ) OR EXISTS (
             SELECT 1
             FROM recipe_ingredient_alternatives ria
             JOIN recipe_ingredient_groups rig ON rig.id = ria.group_id
             JOIN recipes r ON r.id = rig.recipe_id
             WHERE ria.canonical_ingredient_id = c.id
               AND (r.owner_id = ? OR r.id IN (SELECT id FROM public_eligible_recipes))
         ) OR EXISTS (
             SELECT 1 FROM ingredient_aliases a
             WHERE a.canonical_ingredient_id = c.id AND a.owner_id = ?
         )
         LIMIT ?",
    )
    .bind(&user.id)
    .bind(&user.id)
    .bind(&user.id)
    .bind(&user.id)
    .bind(MAX_INGREDIENT_SUGGESTION_CANDIDATES + 1)
    .fetch_all(&state.pool)
    .await?;
    if candidates.len() as i64 > MAX_INGREDIENT_SUGGESTION_CANDIDATES {
        return Err(AppError::Validation(
            "ingredient suggestion candidate limit exceeded".into(),
        ));
    }
    let aliases: Vec<(String, String)> = sqlx::query_as(
        "SELECT canonical_ingredient_id, alias_name
         FROM ingredient_aliases WHERE owner_id = ?
         ORDER BY alias_normalized LIMIT ?",
    )
    .bind(&user.id)
    .bind(MAX_INGREDIENT_ALIASES_PER_OWNER + 1)
    .fetch_all(&state.pool)
    .await?;
    if aliases.len() as i64 > MAX_INGREDIENT_ALIASES_PER_OWNER {
        return Err(AppError::Validation(
            "ingredient alias limit exceeded".into(),
        ));
    }
    let mut alias_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (canonical_id, alias_name) in aliases {
        alias_map.entry(canonical_id).or_default().push(alias_name);
    }
    let mut suggestions = candidates
        .into_iter()
        .filter_map(|(id, display_name, normalized_name)| {
            let aliases = alias_map.get(&id).cloned().unwrap_or_default();
            let reason = match_reason(&query_text, &normalized_name, &aliases)?;
            let matched_text =
                matched_suggestion_text(&query_text, &display_name, &aliases, reason);
            Some(IngredientSuggestion {
                canonical_ingredient_id: id,
                display_name,
                matched_text,
                reason,
            })
        })
        .collect::<Vec<_>>();
    if suggestions
        .iter()
        .any(|suggestion| matches!(suggestion.reason, MatchReason::Exact | MatchReason::Alias))
    {
        suggestions.retain(|suggestion| {
            matches!(suggestion.reason, MatchReason::Exact | MatchReason::Alias)
        });
    }
    suggestions.sort_by(|left, right| {
        left.reason
            .cmp(&right.reason)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    let mut similar_count = 0usize;
    suggestions.retain(|suggestion| {
        if suggestion.reason == MatchReason::Similar {
            similar_count += 1;
            similar_count <= 2
        } else {
            true
        }
    });
    suggestions.truncate(8);
    Ok(Json(suggestions))
}

async fn list_pantry(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<(HeaderMap, Json<Vec<PantryItem>>)> {
    let items = load_pantry(&state.pool, &user.id).await?;
    let digest = Sha256::digest(
        serde_json::to_vec(&items).map_err(|error| AppError::Data(error.to_string()))?,
    );
    let etag = format!("\"pantry-{digest:x}\"").parse().map_err(
        |error: axum::http::header::InvalidHeaderValue| AppError::Data(error.to_string()),
    )?;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::ETAG, etag);
    Ok((headers, Json(items)))
}

async fn upsert_pantry(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(items): Json<Vec<PantryItemInput>>,
) -> AppResult<Json<Vec<PantryItem>>> {
    let items = expand_pantry_inputs(items);
    if items.len() > 100 {
        return Err(AppError::Validation(
            "at most 100 pantry items can be updated".into(),
        ));
    }
    let mut transaction = state.pool.begin().await?;
    let mut pantry_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pantry_items WHERE owner_id = ?")
            .bind(&user.id)
            .fetch_one(&mut *transaction)
            .await?;
    for item in items {
        let normalized = normalize_ingredient(&item.name);
        if normalized.is_empty() {
            return Err(AppError::Validation("pantry item name is required".into()));
        }
        if item.name.chars().count() > 120 || item.quantity_note.chars().count() > 240 {
            return Err(AppError::Validation(
                "pantry names may use 120 characters and notes may use 240".into(),
            ));
        }
        let canonical_id = resolve_canonical_ingredient(
            &mut transaction,
            &user.id,
            &item.name,
            item.canonical_ingredient_id.as_deref(),
        )
        .await?;
        let existing_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM pantry_items
             WHERE owner_id = ? AND (normalized_name = ? OR canonical_ingredient_id = ?)
             ORDER BY CASE WHEN normalized_name = ? THEN 0 ELSE 1 END",
        )
        .bind(&user.id)
        .bind(&normalized)
        .bind(&canonical_id)
        .bind(&normalized)
        .fetch_all(&mut *transaction)
        .await?;
        let item_id = existing_ids
            .first()
            .cloned()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if existing_ids.is_empty() {
            if pantry_count >= MAX_PANTRY_ITEMS_PER_OWNER {
                return Err(AppError::Validation("pantry item limit exceeded".into()));
            }
            pantry_count += 1;
        } else if existing_ids.len() > 1 {
            pantry_count -= (existing_ids.len() - 1) as i64;
        }
        let existing_expiry: Option<String> = if existing_ids.is_empty() {
            None
        } else {
            sqlx::query_scalar("SELECT expires_on FROM pantry_items WHERE id = ?")
                .bind(&item_id)
                .fetch_one(&mut *transaction)
                .await?
        };
        let expires_on = match item.expires_on {
            crate::models::NullableUpdate::Value(value) => {
                validate_expiration_date(value.as_deref())?
            }
            crate::models::NullableUpdate::Missing => existing_expiry,
        };
        let existing_auto_add: bool = if existing_ids.is_empty() {
            false
        } else {
            sqlx::query_scalar(
                "SELECT auto_add_to_shopping_list_when_out FROM pantry_items WHERE id = ?",
            )
            .bind(&item_id)
            .fetch_one(&mut *transaction)
            .await?
        };
        let auto_add_to_shopping_list_when_out = match item.auto_add_to_shopping_list_when_out {
            crate::models::NullableUpdate::Value(value) => value.unwrap_or(false),
            crate::models::NullableUpdate::Missing => existing_auto_add,
        };
        for duplicate_id in existing_ids.iter().skip(1) {
            sqlx::query("DELETE FROM pantry_items WHERE id = ? AND owner_id = ?")
                .bind(duplicate_id)
                .bind(&user.id)
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query(
            "INSERT INTO pantry_items
             (id, owner_id, name, normalized_name, status, quantity_note, confirmed_at,
              expires_on, canonical_ingredient_id, auto_add_to_shopping_list_when_out)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name, normalized_name = excluded.normalized_name,
               status = excluded.status, quantity_note = excluded.quantity_note,
               confirmed_at = excluded.confirmed_at, expires_on = excluded.expires_on,
               canonical_ingredient_id = excluded.canonical_ingredient_id,
               auto_add_to_shopping_list_when_out = excluded.auto_add_to_shopping_list_when_out,
               version = pantry_items.version + 1,
               updated_at = CURRENT_TIMESTAMP",
        )
        .bind(item_id)
        .bind(&user.id)
        .bind(clean_display_name(&item.name))
        .bind(normalized)
        .bind(item.status.as_str())
        .bind(item.quantity_note.trim())
        .bind(Utc::now().to_rfc3339())
        .bind(expires_on)
        .bind(canonical_id)
        .bind(auto_add_to_shopping_list_when_out)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Json(load_pantry(&state.pool, &user.id).await?))
}

async fn get_settings(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<RecommendationSettings>> {
    Ok(Json(load_settings(&state.pool, &user.id).await?))
}

async fn update_settings(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(settings): Json<RecommendationSettings>,
) -> AppResult<Json<RecommendationSettings>> {
    if !(1..=90).contains(&settings.verification_days) {
        return Err(AppError::Validation(
            "verificationDays must be between 1 and 90".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO recommendation_settings
         (owner_id, include_favorites, include_own, include_public,
          include_favorite_accounts, verification_days)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(owner_id) DO UPDATE SET include_favorites = excluded.include_favorites,
           include_own = excluded.include_own, include_public = excluded.include_public,
           include_favorite_accounts = excluded.include_favorite_accounts,
           verification_days = excluded.verification_days",
    )
    .bind(&user.id)
    .bind(settings.include_favorite_recipes)
    .bind(settings.include_own)
    .bind(settings.include_other_public)
    .bind(settings.include_favorite_accounts)
    .bind(settings.verification_days)
    .execute(&state.pool)
    .await?;
    Ok(Json(settings))
}

async fn get_recommendations(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Query(query): Query<RecommendationQuery>,
    Json(mut request): Json<RecommendationRequest>,
) -> AppResult<Json<Vec<Recommendation>>> {
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(AppError::Validation(
            "recommendation limit must be between 1 and 100".into(),
        ));
    }
    enforce_public_api_rate_limit(
        &state.pool,
        &format!(
            "recommendations:{}:{}",
            user.id,
            rate_limit_key(&headers, &state.config.oauth_state_digest_key)
        ),
        60,
    )
    .await?;
    let recommendation_time = if let Some(today) = request.today.as_deref() {
        parse_date_only(today, "today")?
            .and_hms_opt(12, 0, 0)
            .expect("noon is a valid time")
            .and_utc()
    } else {
        Utc::now()
    };
    request.ingredient_scope = Some(match (request.ingredient_scope, request.pantry_only) {
        (Some(_), Some(_)) => {
            return Err(AppError::Validation(
                "ingredientScope and deprecated pantryOnly cannot be supplied together".into(),
            ));
        }
        (Some(scope), None) => scope,
        (None, Some(true)) => crate::models::IngredientScope::PantryOnly,
        (None, Some(false) | None) => crate::models::IngredientScope::Any,
    });
    request.preferred = expand_ingredient_references(request.preferred);
    request.required = expand_ingredient_references(request.required);
    request.excluded = expand_ingredient_references(request.excluded);
    for references in [&request.preferred, &request.required, &request.excluded] {
        if references.len() > 100
            || references
                .iter()
                .any(|reference| reference.name().chars().count() > 120)
        {
            return Err(AppError::Validation(
                "recommendation filters may contain at most 100 ingredients of 120 characters each"
                    .into(),
            ));
        }
    }
    validate_recommendation_request(&state.pool, &user.id, &request).await?;
    let mut settings = load_settings(&state.pool, &user.id).await?;
    settings.include_favorite_recipes = request
        .include_favorite_recipes
        .unwrap_or(settings.include_favorite_recipes);
    settings.include_own = request.include_own.unwrap_or(settings.include_own);
    settings.include_favorite_accounts = request
        .include_favorite_accounts
        .unwrap_or(settings.include_favorite_accounts);
    settings.include_other_public = request
        .include_other_public
        .unwrap_or(settings.include_other_public);
    let mut transaction = state.pool.begin().await?;
    let pantry = load_pantry_tx(&mut transaction, &user.id).await?;
    let active_shopping = shopping::active_coverage_tx(&mut transaction, &user.id).await?;
    let candidates = load_candidate_recipes(&mut transaction, &user.id, &settings).await?;
    pause_after_recommendation_selection(&user.id).await;
    let mut recommendations = recommend_limited_with_shopping(
        candidates,
        &pantry,
        &active_shopping,
        &request,
        &settings,
        recommendation_time,
        limit,
    );
    hydrate_recommendation_recipes(&mut transaction, &user.id, &mut recommendations).await?;
    transaction.commit().await?;
    discard_unauthorized_recommendations(&state.pool, &user.id, &mut recommendations).await?;
    Ok(Json(recommendations))
}

#[derive(Debug, Deserialize)]
struct RecommendationQuery {
    limit: Option<usize>,
}

#[cfg(test)]
#[derive(Clone)]
struct RecommendationSelectionHook {
    user_id: String,
    selected: std::sync::Arc<tokio::sync::Notify>,
    resume: std::sync::Arc<tokio::sync::Notify>,
}

#[cfg(test)]
static RECOMMENDATION_SELECTION_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<RecommendationSelectionHook>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn install_recommendation_selection_hook(user_id: String) -> RecommendationSelectionHook {
    let hook = RecommendationSelectionHook {
        user_id,
        selected: std::sync::Arc::new(tokio::sync::Notify::new()),
        resume: std::sync::Arc::new(tokio::sync::Notify::new()),
    };
    *RECOMMENDATION_SELECTION_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("recommendation test hook mutex poisoned") = Some(hook.clone());
    hook
}

#[cfg(test)]
async fn pause_after_recommendation_selection(user_id: &str) {
    let hook = {
        let mut guard = RECOMMENDATION_SELECTION_HOOK
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .expect("recommendation test hook mutex poisoned");
        let matches_user = guard.as_ref().is_some_and(|hook| hook.user_id == user_id);
        matches_user.then(|| guard.take()).flatten()
    };
    if let Some(hook) = hook {
        hook.selected.notify_one();
        hook.resume.notified().await;
    }
}

#[cfg(not(test))]
async fn pause_after_recommendation_selection(_user_id: &str) {}

fn seconds_until(expires_at: chrono::DateTime<Utc>) -> u64 {
    u64::try_from((expires_at - Utc::now()).num_seconds().max(0)).unwrap_or(0)
}

pub(crate) fn rate_limit_key(headers: &HeaderMap, key: &[u8; 32]) -> String {
    let client = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    let mut digest = Sha256::new();
    digest.update(key);
    digest.update(client.as_bytes());
    format!("{:x}", digest.finalize())
}

fn map_login_error(error: LoginError) -> AppError {
    match error {
        LoginError::RateLimited => AppError::RateLimited,
        LoginError::ProviderUnavailable => AppError::AuthUnavailable,
        LoginError::InvalidConfiguration | LoginError::Internal => AppError::AuthUnavailable,
        LoginError::InvalidAttempt
        | LoginError::AccessDenied
        | LoginError::TokenRejected
        | LoginError::IdentityIncomplete => AppError::Unauthorized,
    }
}

pub(crate) fn validate_recipe(input: &RecipeInput) -> AppResult<()> {
    if input.title.trim().is_empty() {
        return Err(AppError::Validation("title is required".into()));
    }
    if input.title.trim().chars().count() > 120
        || input.title.chars().any(char::is_control)
        || input.description.chars().count() > 2_000
        || input.instructions.chars().count() > 20_000
    {
        return Err(AppError::Validation(
            "recipe title, description, or instructions exceed the allowed length".into(),
        ));
    }
    validate_ingredient_groups(&effective_ingredient_groups(input)?)?;
    if input.visibility != "private" && input.visibility != "public" {
        return Err(AppError::Validation(
            "visibility must be private or public".into(),
        ));
    }
    if input
        .difficulty
        .is_some_and(|value| !(1..=5).contains(&value))
    {
        return Err(AppError::Validation(
            "difficulty must be an integer from 1 to 5".into(),
        ));
    }
    if input
        .estimated_calories
        .is_some_and(|value| !(1..=100_000).contains(&value))
    {
        return Err(AppError::Validation(
            "estimated total calories must be an integer from 1 to 100000".into(),
        ));
    }
    validate_recipe_source(input)?;
    Ok(())
}

pub(crate) fn effective_ingredient_groups(
    input: &RecipeInput,
) -> AppResult<Vec<IngredientGroupInput>> {
    if input.ingredient_groups.is_some() && !input.ingredients.is_empty() {
        return Err(AppError::Validation(
            "ingredients and ingredientGroups cannot both be provided".into(),
        ));
    }
    if let Some(groups) = &input.ingredient_groups {
        if input.ingredient_model.as_deref() != Some("groups-v1") {
            return Err(AppError::Validation(
                "ingredientModel must be groups-v1 when ingredientGroups is provided".into(),
            ));
        }
        return Ok(groups.clone());
    }
    if input.ingredient_model.is_some() {
        return Err(AppError::Validation(
            "ingredientModel is only valid with ingredientGroups".into(),
        ));
    }
    Ok(expand_ingredient_inputs(input.ingredients.clone())
        .into_iter()
        .enumerate()
        .map(|(index, ingredient)| IngredientGroupInput {
            id: Uuid::new_v4().to_string(),
            optional: false,
            note: String::new(),
            sort_order: index as i64,
            alternatives: vec![IngredientAlternativeInput {
                id: Uuid::new_v4().to_string(),
                name: ingredient.name,
                quantity_note: ingredient.quantity_note,
                canonical_ingredient_id: ingredient.canonical_ingredient_id,
                sort_order: 0,
            }],
        })
        .collect())
}

pub(crate) fn validate_ingredient_groups(groups: &[IngredientGroupInput]) -> AppResult<()> {
    if groups.is_empty() || groups.len() > 100 {
        return Err(AppError::Validation(
            "ingredientGroups must contain between 1 and 100 groups".into(),
        ));
    }
    if groups
        .iter()
        .map(|group| group.alternatives.len())
        .sum::<usize>()
        > 100
    {
        return Err(AppError::Validation(
            "ingredientGroups may contain at most 100 alternatives".into(),
        ));
    }
    let mut group_ids = std::collections::HashSet::new();
    let mut group_orders = std::collections::HashSet::new();
    for group in groups {
        if group.id.trim().is_empty()
            || group.id.chars().count() > 120
            || !group_ids.insert(group.id.clone())
        {
            return Err(AppError::Validation(
                "ingredient group ids must be non-empty, unique, and at most 120 characters".into(),
            ));
        }
        if group.sort_order < 0 || !group_orders.insert(group.sort_order) {
            return Err(AppError::Validation(
                "ingredient group sortOrder values must be non-negative and unique".into(),
            ));
        }
        if group.note.chars().count() > 240 {
            return Err(AppError::Validation(
                "ingredient group notes may use at most 240 characters".into(),
            ));
        }
        if group.alternatives.is_empty() || group.alternatives.len() > 8 {
            return Err(AppError::Validation(
                "each ingredient group needs between 1 and 8 alternatives".into(),
            ));
        }
        let mut alternative_ids = std::collections::HashSet::new();
        let mut alternative_orders = std::collections::HashSet::new();
        let mut identities = std::collections::HashSet::new();
        for alternative in &group.alternatives {
            if alternative.id.trim().is_empty()
                || alternative.id.chars().count() > 120
                || !alternative_ids.insert(alternative.id.clone())
            {
                return Err(AppError::Validation(
                    "ingredient alternative ids must be non-empty, unique within a group, and at most 120 characters".into(),
                ));
            }
            if alternative.sort_order < 0 || !alternative_orders.insert(alternative.sort_order) {
                return Err(AppError::Validation(
                    "ingredient alternative sortOrder values must be non-negative and unique within a group".into(),
                ));
            }
            if alternative.name.trim().is_empty()
                || alternative.name.chars().count() > 120
                || alternative.quantity_note.chars().count() > 240
            {
                return Err(AppError::Validation(
                    "ingredient alternatives require a name of at most 120 characters and a quantity note of at most 240 characters".into(),
                ));
            }
            let identity = alternative
                .canonical_ingredient_id
                .as_deref()
                .map(|id| format!("id:{id}"))
                .unwrap_or_else(|| format!("name:{}", normalize_ingredient(&alternative.name)));
            if !identities.insert(identity) {
                return Err(AppError::Validation(
                    "ingredient alternatives must be unique by canonical or unresolved identity within a group".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_recipe_source(input: &RecipeInput) -> AppResult<()> {
    if let Some(value) = optional_text(&input.source_url) {
        if value.len() > 2_048 {
            return Err(AppError::Validation(
                "source URL must be at most 2048 characters".into(),
            ));
        }
        let url = Url::parse(value)
            .map_err(|_| AppError::Validation("source URL must be HTTP or HTTPS".into()))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(AppError::Validation(
                "source URL must be HTTP or HTTPS".into(),
            ));
        }
    }
    for (label, value) in [
        ("source title", &input.source_title),
        ("source author", &input.source_author),
        ("source license", &input.source_license),
    ] {
        if optional_text(value).is_some_and(|text| text.chars().count() > 200) {
            return Err(AppError::Validation(format!(
                "{label} must be at most 200 characters"
            )));
        }
    }
    if optional_text(&input.source_attribution).is_some_and(|text| text.chars().count() > 1_000) {
        return Err(AppError::Validation(
            "source attribution must be at most 1000 characters".into(),
        ));
    }
    Ok(())
}

pub(crate) fn optional_text(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn expand_ingredient_inputs(items: Vec<IngredientInput>) -> Vec<IngredientInput> {
    let mut expanded = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in items {
        let names = split_ingredient_list(&item.name);
        let single = names.len() == 1;
        for name in names {
            let key = normalize_ingredient(&name);
            if seen.insert(key) {
                expanded.push(IngredientInput {
                    name,
                    quantity_note: item.quantity_note.clone(),
                    canonical_ingredient_id: single
                        .then(|| item.canonical_ingredient_id.clone())
                        .flatten(),
                });
            }
        }
    }
    expanded
}

fn expand_ingredient_references(
    references: Vec<crate::models::IngredientReference>,
) -> Vec<crate::models::IngredientReference> {
    let mut expanded = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for reference in references {
        let names = split_ingredient_list(reference.name());
        let single = names.len() == 1;
        for name in names {
            let key = normalize_ingredient(&name);
            if seen.insert(key) {
                expanded.push(crate::models::IngredientReference::Selection {
                    name,
                    canonical_ingredient_id: single
                        .then(|| reference.canonical_ingredient_id().map(str::to_string))
                        .flatten(),
                });
            }
        }
    }
    expanded
}

fn expand_pantry_inputs(items: Vec<PantryItemInput>) -> Vec<PantryItemInput> {
    let mut expanded = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in items {
        let names = split_ingredient_list(&item.name);
        let single = names.len() == 1;
        for name in names {
            let key = normalize_ingredient(&name);
            if seen.insert(key) {
                expanded.push(PantryItemInput {
                    name,
                    status: item.status,
                    quantity_note: item.quantity_note.clone(),
                    expires_on: item.expires_on.clone(),
                    canonical_ingredient_id: single
                        .then(|| item.canonical_ingredient_id.clone())
                        .flatten(),
                    auto_add_to_shopping_list_when_out: item
                        .auto_add_to_shopping_list_when_out
                        .clone(),
                });
            }
        }
    }
    expanded
}

fn validate_expiration_date(value: Option<&str>) -> AppResult<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    parse_date_only(value, "expiresOn")?;
    Ok(Some(value.to_string()))
}

fn parse_date_only(value: &str, field: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| AppError::Validation(format!("{field} must use a real YYYY-MM-DD date")))
}

async fn resolve_canonical_ingredient(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_id: &str,
    display_name: &str,
    selected_id: Option<&str>,
) -> AppResult<String> {
    let display_name = clean_display_name(display_name);
    let normalized = normalize_ingredient(&display_name);
    if normalized.is_empty() {
        return Err(AppError::Validation("ingredient name is required".into()));
    }
    let canonical_id = if let Some(selected_id) = selected_id {
        if !canonical_ingredient_is_visible(transaction, owner_id, selected_id).await? {
            return Err(AppError::Validation(
                "selected canonical ingredient is not available to this account".into(),
            ));
        }
        selected_id.to_string()
    } else {
        let exact_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM ingredient_catalog WHERE normalized_name = ?")
                .bind(&normalized)
                .fetch_optional(&mut **transaction)
                .await?;
        let alias_id: Option<String> = sqlx::query_scalar(
            "SELECT canonical_ingredient_id FROM ingredient_aliases
             WHERE owner_id = ? AND alias_normalized = ?",
        )
        .bind(owner_id)
        .bind(&normalized)
        .fetch_optional(&mut **transaction)
        .await?;
        if exact_id.is_some() && alias_id.is_some() && exact_id != alias_id {
            return Err(AppError::Validation(
                "ingredient identity is ambiguous; select canonicalIngredientId".into(),
            ));
        }
        if let Some(id) = exact_id.or(alias_id) {
            id
        } else {
            sqlx::query(
                "INSERT INTO ingredient_catalog
             (id, normalized_name, display_name, created_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(normalized_name) DO NOTHING",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&normalized)
            .bind(&display_name)
            .bind(Utc::now().timestamp())
            .execute(&mut **transaction)
            .await?;
            sqlx::query_scalar("SELECT id FROM ingredient_catalog WHERE normalized_name = ?")
                .bind(&normalized)
                .fetch_one(&mut **transaction)
                .await?
        }
    };
    if selected_id.is_some() {
        let (canonical_normalized, canonical_display): (String, String) = sqlx::query_as(
            "SELECT normalized_name, display_name FROM ingredient_catalog WHERE id = ?",
        )
        .bind(&canonical_id)
        .fetch_one(&mut **transaction)
        .await?;
        if canonical_normalized != normalized {
            let advisory = match_reason(&display_name, &canonical_display, &[]);
            let alias_exists: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM ingredient_aliases
                 WHERE owner_id = ? AND alias_normalized = ?",
            )
            .bind(owner_id)
            .bind(&normalized)
            .fetch_optional(&mut **transaction)
            .await?;
            if alias_exists.is_none()
                && !matches!(
                    advisory,
                    Some(MatchReason::Prefix | MatchReason::Substring | MatchReason::Similar)
                )
            {
                let alias_count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM ingredient_aliases WHERE owner_id = ?",
                )
                .bind(owner_id)
                .fetch_one(&mut **transaction)
                .await?;
                if alias_count >= MAX_INGREDIENT_ALIASES_PER_OWNER {
                    return Err(AppError::Validation(
                        "ingredient alias limit exceeded".into(),
                    ));
                }
            }
            if !matches!(
                advisory,
                Some(MatchReason::Prefix | MatchReason::Substring | MatchReason::Similar)
            ) {
                sqlx::query(
                    "INSERT INTO ingredient_aliases
                 (id, owner_id, alias_name, alias_normalized, canonical_ingredient_id, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT(owner_id, alias_normalized) DO UPDATE SET
                   alias_name = excluded.alias_name,
                   canonical_ingredient_id = excluded.canonical_ingredient_id",
                )
                .bind(Uuid::new_v4().to_string())
                .bind(owner_id)
                .bind(&display_name)
                .bind(&normalized)
                .bind(&canonical_id)
                .bind(Utc::now().timestamp())
                .execute(&mut **transaction)
                .await?;
            }
        }
    }
    Ok(canonical_id)
}

async fn canonical_ingredient_is_visible(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_id: &str,
    canonical_id: &str,
) -> AppResult<bool> {
    let visible: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM ingredient_catalog c
         WHERE c.id = ? AND (
           EXISTS (
             SELECT 1 FROM pantry_items p
             WHERE p.canonical_ingredient_id = c.id AND p.owner_id = ?
           ) OR EXISTS (
             SELECT 1 FROM recipe_ingredients ri
             JOIN recipes r ON r.id = ri.recipe_id
             WHERE ri.canonical_ingredient_id = c.id
               AND (r.owner_id = ? OR EXISTS (
                 SELECT 1 FROM public_eligible_recipes p WHERE p.id = r.id
               ))
           ) OR EXISTS (
             SELECT 1 FROM recipe_ingredient_alternatives ria
             JOIN recipe_ingredient_groups rig ON rig.id = ria.group_id
             JOIN recipes r ON r.id = rig.recipe_id
             WHERE ria.canonical_ingredient_id = c.id
               AND (r.owner_id = ? OR EXISTS (
                 SELECT 1 FROM public_eligible_recipes p WHERE p.id = r.id
               ))
           ) OR EXISTS (
             SELECT 1 FROM ingredient_aliases a
             WHERE a.canonical_ingredient_id = c.id AND a.owner_id = ?
           )
         )",
    )
    .bind(canonical_id)
    .bind(owner_id)
    .bind(owner_id)
    .bind(owner_id)
    .bind(owner_id)
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(visible.is_some())
}

pub(crate) async fn validate_ingredient_selections(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_id: &str,
    ingredients: &[IngredientInput],
) -> AppResult<()> {
    for ingredient in ingredients {
        if let Some(selected_id) = ingredient.canonical_ingredient_id.as_deref() {
            if !canonical_ingredient_is_visible(transaction, owner_id, selected_id).await? {
                return Err(AppError::Validation(
                    "selected canonical ingredient is not available to this account".into(),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) async fn validate_group_selections(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_id: &str,
    groups: &[IngredientGroupInput],
) -> AppResult<()> {
    for group in groups {
        for alternative in &group.alternatives {
            if let Some(selected_id) = alternative.canonical_ingredient_id.as_deref() {
                if !canonical_ingredient_is_visible(transaction, owner_id, selected_id).await? {
                    return Err(AppError::Validation(
                        "selected canonical ingredient is not available to this account".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

async fn validate_recommendation_request(
    pool: &SqlitePool,
    owner_id: &str,
    request: &RecommendationRequest,
) -> AppResult<()> {
    let mut unavailable = Vec::new();
    for references in [&request.preferred, &request.required, &request.excluded] {
        for reference in references.iter() {
            if let Some(canonical_ingredient_id) = reference.canonical_ingredient_id() {
                if canonical_ingredient_id.is_empty()
                    || !canonical_ingredient_is_visible_pool(
                        pool,
                        owner_id,
                        canonical_ingredient_id,
                    )
                    .await?
                {
                    unavailable.push(canonical_ingredient_id.to_string());
                }
            }
        }
    }
    if !unavailable.is_empty() {
        unavailable.sort();
        unavailable.dedup();
        return Err(AppError::CanonicalIngredientsUnavailable {
            canonical_ingredient_ids: unavailable,
        });
    }
    Ok(())
}

async fn canonical_ingredient_is_visible_pool(
    pool: &SqlitePool,
    owner_id: &str,
    canonical_id: &str,
) -> AppResult<bool> {
    let visible: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM ingredient_catalog c
         WHERE c.id = ? AND (
           EXISTS (SELECT 1 FROM pantry_items p
                   WHERE p.canonical_ingredient_id = c.id AND p.owner_id = ?)
           OR EXISTS (
             SELECT 1 FROM recipe_ingredients ri JOIN recipes r ON r.id = ri.recipe_id
             WHERE ri.canonical_ingredient_id = c.id
               AND (r.owner_id = ? OR EXISTS (
                 SELECT 1 FROM public_eligible_recipes p WHERE p.id = r.id
               ))
           )
           OR EXISTS (
             SELECT 1 FROM recipe_ingredient_alternatives ria
             JOIN recipe_ingredient_groups rig ON rig.id = ria.group_id
             JOIN recipes r ON r.id = rig.recipe_id
             WHERE ria.canonical_ingredient_id = c.id
               AND (r.owner_id = ? OR EXISTS (
                 SELECT 1 FROM public_eligible_recipes p WHERE p.id = r.id
               ))
           )
           OR EXISTS (SELECT 1 FROM ingredient_aliases a
                      WHERE a.canonical_ingredient_id = c.id AND a.owner_id = ?)
         )",
    )
    .bind(canonical_id)
    .bind(owner_id)
    .bind(owner_id)
    .bind(owner_id)
    .bind(owner_id)
    .fetch_optional(pool)
    .await?;
    Ok(visible.is_some())
}

fn matched_suggestion_text(
    query: &str,
    canonical: &str,
    aliases: &[String],
    reason: MatchReason,
) -> String {
    if reason != MatchReason::Alias && match_reason(query, canonical, &[]) == Some(reason) {
        return canonical.to_string();
    }
    aliases
        .iter()
        .find(|alias| {
            reason == MatchReason::Alias
                && normalize_ingredient(query) == normalize_ingredient(alias)
        })
        .cloned()
        .unwrap_or_else(|| canonical.to_string())
}

async fn save_recipe(
    pool: &SqlitePool,
    id: &str,
    owner_id: &str,
    input: &RecipeInput,
    imported: bool,
) -> AppResult<()> {
    let _ = imported;
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO recipes
         (id, owner_id, title, description, instructions, visibility, publication_intent,
          difficulty,
          estimated_calories, source_url, source_title, source_author, source_license,
          source_attribution)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(owner_id)
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(input.instructions.trim())
    .bind(&input.visibility)
    .bind(&input.visibility)
    .bind(input.difficulty)
    .bind(input.estimated_calories)
    .bind(optional_text(&input.source_url))
    .bind(optional_text(&input.source_title))
    .bind(optional_text(&input.source_author))
    .bind(optional_text(&input.source_license))
    .bind(optional_text(&input.source_attribution))
    .execute(&mut *transaction)
    .await?;
    let groups = effective_ingredient_groups(input)?;
    insert_ingredient_groups(&mut transaction, id, owner_id, &groups).await?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn replace_ingredient_groups(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recipe_id: &str,
    owner_id: &str,
    groups: &[IngredientGroupInput],
) -> AppResult<()> {
    let mut groups = groups.to_vec();
    for group in &mut groups {
        for alternative in &mut group.alternatives {
            alternative.canonical_ingredient_id = Some(
                resolve_canonical_ingredient(
                    transaction,
                    owner_id,
                    &alternative.name,
                    alternative.canonical_ingredient_id.as_deref(),
                )
                .await?,
            );
        }
    }
    sqlx::query("DELETE FROM recipe_ingredient_groups WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM recipe_ingredients WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut **transaction)
        .await?;
    insert_ingredient_groups_inner(transaction, recipe_id, owner_id, &groups, true).await
}

pub(crate) async fn insert_ingredient_groups(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recipe_id: &str,
    owner_id: &str,
    groups: &[IngredientGroupInput],
) -> AppResult<()> {
    insert_ingredient_groups_inner(transaction, recipe_id, owner_id, groups, false).await
}

async fn insert_ingredient_groups_inner(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recipe_id: &str,
    owner_id: &str,
    groups: &[IngredientGroupInput],
    canonical_ids_prevalidated: bool,
) -> AppResult<()> {
    validate_ingredient_groups(groups)?;
    let mut ordered_groups = groups.iter().collect::<Vec<_>>();
    ordered_groups.sort_by_key(|group| group.sort_order);
    for group in ordered_groups {
        let group_sort = group.sort_order;
        let persisted_group_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO recipe_ingredient_groups
             (id, recipe_id, external_id, optional, note, sort_order)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&persisted_group_id)
        .bind(recipe_id)
        .bind(&group.id)
        .bind(group.optional)
        .bind(group.note.trim())
        .bind(group_sort)
        .execute(&mut **transaction)
        .await?;
        let mut resolved = Vec::with_capacity(group.alternatives.len());
        let mut ordered_alternatives = group.alternatives.iter().collect::<Vec<_>>();
        ordered_alternatives.sort_by_key(|alternative| alternative.sort_order);
        for alternative in ordered_alternatives {
            let canonical_id = if canonical_ids_prevalidated {
                alternative
                    .canonical_ingredient_id
                    .clone()
                    .ok_or_else(|| AppError::Data("resolved ingredient is missing an id".into()))?
            } else {
                resolve_canonical_ingredient(
                    transaction,
                    owner_id,
                    &alternative.name,
                    alternative.canonical_ingredient_id.as_deref(),
                )
                .await?
            };
            sqlx::query(
                "INSERT INTO recipe_ingredient_alternatives
                 (id, group_id, external_id, name, normalized_name, quantity_note,
                  canonical_ingredient_id, sort_order)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&persisted_group_id)
            .bind(&alternative.id)
            .bind(clean_display_name(&alternative.name))
            .bind(normalize_ingredient(&alternative.name))
            .bind(alternative.quantity_note.trim())
            .bind(&canonical_id)
            .bind(alternative.sort_order)
            .execute(&mut **transaction)
            .await?;
            resolved.push((alternative, canonical_id));
        }
        let display_name = resolved
            .iter()
            .map(|(alternative, _)| clean_display_name(&alternative.name))
            .collect::<Vec<_>>()
            .join(" / ");
        let display_canonical = (resolved.len() == 1).then(|| resolved[0].1.clone());
        sqlx::query(
            "INSERT INTO recipe_ingredients
             (id, recipe_id, name, normalized_name, quantity_note, sort_order,
              canonical_ingredient_id)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(recipe_id)
        .bind(&display_name)
        .bind(normalize_ingredient(&display_name))
        .bind(if resolved.len() == 1 {
            resolved[0].0.quantity_note.trim()
        } else {
            group.note.trim()
        })
        .bind(group_sort)
        .bind(display_canonical)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

pub(crate) async fn load_visible_recipes(
    pool: &SqlitePool,
    user_id: &str,
    id: Option<&str>,
) -> AppResult<Vec<Recipe>> {
    let rows = if let Some(id) = id {
        sqlx::query(
            "SELECT r.id, r.owner_id, r.title, r.description, r.instructions, r.visibility,
               r.publication_intent, r.moderation_state, r.moderation_review_state,
               r.moderation_version,
               r.difficulty, r.estimated_calories, r.version,
               u.public_handle, u.public_nickname, r.source_url, r.source_title, r.source_author,
               r.source_license, r.source_attribution, r.source_warnings, r.source_confidence,
               EXISTS(SELECT 1 FROM favorites f WHERE f.user_id = ? AND f.recipe_id = r.id) AS is_favorite,
               EXISTS(SELECT 1 FROM favorite_accounts fa
                      WHERE fa.viewer_id = ? AND fa.author_id = r.owner_id) AS is_favorite_author
             FROM recipes r JOIN users u ON u.id = r.owner_id
             WHERE r.id = ? AND (
               r.owner_id = ? OR r.id IN (SELECT id FROM public_eligible_recipes)
             )",
        )
        .bind(user_id)
        .bind(user_id)
        .bind(id)
        .bind(user_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT r.id, r.owner_id, r.title, r.description, r.instructions, r.visibility,
               r.publication_intent, r.moderation_state, r.moderation_review_state,
               r.moderation_version,
               r.difficulty, r.estimated_calories, r.version,
               u.public_handle, u.public_nickname, r.source_url, r.source_title, r.source_author,
               r.source_license, r.source_attribution, r.source_warnings, r.source_confidence,
               EXISTS(SELECT 1 FROM favorites f WHERE f.user_id = ? AND f.recipe_id = r.id) AS is_favorite,
               EXISTS(SELECT 1 FROM favorite_accounts fa
                      WHERE fa.viewer_id = ? AND fa.author_id = r.owner_id) AS is_favorite_author
             FROM recipes r JOIN users u ON u.id = r.owner_id
             WHERE r.owner_id = ? OR r.id IN (SELECT id FROM public_eligible_recipes)
             ORDER BY r.updated_at DESC",
        )
        .bind(user_id)
        .bind(user_id)
        .bind(user_id)
        .fetch_all(pool)
        .await?
    };
    let mut recipes = Vec::with_capacity(rows.len());
    for row in rows {
        let recipe_id: String = row.get("id");
        let owner_id: String = row.get("owner_id");
        let source_url: Option<String> = row.get("source_url");
        let source_title: Option<String> = row.get("source_title");
        let source_author: Option<String> = row.get("source_author");
        let source_license: Option<String> = row.get("source_license");
        let source_attribution: Option<String> = row.get("source_attribution");
        let source = if source_url.is_some()
            || source_title.is_some()
            || source_author.is_some()
            || source_license.is_some()
            || source_attribution.is_some()
        {
            let warnings = match row.get::<Option<String>, _>("source_warnings") {
                Some(value) => serde_json::from_str(&value).map_err(|error| {
                    AppError::Data(format!("invalid stored recipe source warnings: {error}"))
                })?,
                None => Vec::new(),
            };
            Some(RecipeSource {
                url: source_url,
                title: source_title,
                author: source_author,
                license: source_license,
                attribution: source_attribution,
                warnings,
                confidence: row.get("source_confidence"),
            })
        } else {
            None
        };
        let ingredients = load_ingredients(pool, &recipe_id).await?;
        let mut ingredient_groups = load_ingredient_groups(pool, &recipe_id).await?;
        if ingredient_groups.is_empty() {
            ingredient_groups = legacy_projection_groups(&recipe_id, &ingredients);
        }
        recipes.push(Recipe {
            ingredient_model: "groups-v1".into(),
            ingredient_groups,
            ingredients,
            cover_image: load_recipe_image_meta(pool, &recipe_id).await?,
            id: recipe_id,
            is_owner: owner_id == user_id,
            owner_id,
            author_handle: row.get("public_handle"),
            author_nickname: row.get("public_nickname"),
            title: row.get("title"),
            description: row.get("description"),
            instructions: row.get("instructions"),
            visibility: row.get("visibility"),
            publication_intent: row.get("publication_intent"),
            moderation_state: row.get("moderation_state"),
            moderation_review_state: row.get("moderation_review_state"),
            moderation_version: row.get("moderation_version"),
            difficulty: row.get("difficulty"),
            estimated_calories: row.get("estimated_calories"),
            version: row.get("version"),
            is_favorite: row.get::<i64, _>("is_favorite") != 0,
            is_favorite_author: row.get::<i64, _>("is_favorite_author") != 0,
            source,
        });
    }

    Ok(recipes)
}

pub(crate) async fn load_recipe_image_meta(
    pool: &SqlitePool,
    recipe_id: &str,
) -> AppResult<Option<RecipeImage>> {
    let row = sqlx::query(
        "SELECT id, width, height, alt_text, source_url, source_license,
                source_attribution, version
         FROM recipe_images WHERE recipe_id = ?",
    )
    .bind(recipe_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| {
        let id: String = row.get("id");
        RecipeImage {
            url: format!("/api/recipe-images/{id}"),
            id,
            width: row.get("width"),
            height: row.get("height"),
            alt_text: row.get("alt_text"),
            source_url: row.get("source_url"),
            source_license: row.get("source_license"),
            source_attribution: row.get("source_attribution"),
            version: row.get("version"),
        }
    }))
}

async fn load_candidate_recipes(
    transaction: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    settings: &RecommendationSettings,
) -> AppResult<Vec<Recipe>> {
    let rows = sqlx::query(
        "SELECT r.id, r.owner_id, r.title, r.visibility, r.difficulty,
                r.estimated_calories, r.version, r.publication_intent, r.moderation_state,
                r.moderation_review_state, r.moderation_version,
                u.public_handle, u.public_nickname,
                EXISTS(SELECT 1 FROM favorites f
                       WHERE f.user_id = ? AND f.recipe_id = r.id) AS is_favorite,
                EXISTS(SELECT 1 FROM favorite_accounts fa
                       WHERE fa.viewer_id = ? AND fa.author_id = r.owner_id) AS is_favorite_author
         FROM recipes r
         JOIN users u ON u.id = r.owner_id
         WHERE (? AND r.owner_id = ?)
            OR (? AND EXISTS(SELECT 1 FROM favorites f
                             WHERE f.user_id = ? AND f.recipe_id = r.id)
                    AND (r.owner_id = ? OR r.id IN (SELECT id FROM public_eligible_recipes)))
            OR (? AND r.id IN (SELECT id FROM public_eligible_recipes)
                    AND EXISTS(SELECT 1 FROM favorite_accounts fa
                               WHERE fa.viewer_id = ? AND fa.author_id = r.owner_id))
            OR (? AND r.owner_id <> ? AND r.id IN (SELECT id FROM public_eligible_recipes))
         ORDER BY r.id
         LIMIT ?",
    )
    .bind(user_id)
    .bind(user_id)
    .bind(settings.include_own)
    .bind(user_id)
    .bind(settings.include_favorite_recipes)
    .bind(user_id)
    .bind(user_id)
    .bind(settings.include_favorite_accounts)
    .bind(user_id)
    .bind(settings.include_other_public)
    .bind(user_id)
    .bind(MAX_RECOMMENDATION_CANDIDATES + 1)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() as i64 > MAX_RECOMMENDATION_CANDIDATES {
        return Err(AppError::PayloadTooLarge(
            "recommendation candidate limit exceeded".into(),
        ));
    }
    let ingredient_rows = sqlx::query(
        "WITH candidates AS (
           SELECT r.id
           FROM recipes r
           WHERE (? AND r.owner_id = ?)
              OR (? AND EXISTS(SELECT 1 FROM favorites f
                               WHERE f.user_id = ? AND f.recipe_id = r.id)
                      AND (r.owner_id = ? OR r.id IN (SELECT id FROM public_eligible_recipes)))
              OR (? AND r.id IN (SELECT id FROM public_eligible_recipes)
                      AND EXISTS(SELECT 1 FROM favorite_accounts fa
                                 WHERE fa.viewer_id = ? AND fa.author_id = r.owner_id))
              OR (? AND r.owner_id <> ? AND r.id IN (SELECT id FROM public_eligible_recipes))
           ORDER BY r.id
           LIMIT ?
         )
         SELECT ri.recipe_id, ri.name, ri.quantity_note, ri.canonical_ingredient_id
         FROM recipe_ingredients ri
         JOIN candidates c ON c.id = ri.recipe_id
         ORDER BY ri.recipe_id, ri.sort_order
         LIMIT ?",
    )
    .bind(settings.include_own)
    .bind(user_id)
    .bind(settings.include_favorite_recipes)
    .bind(user_id)
    .bind(user_id)
    .bind(settings.include_favorite_accounts)
    .bind(user_id)
    .bind(settings.include_other_public)
    .bind(user_id)
    .bind(MAX_RECOMMENDATION_CANDIDATES)
    .bind(MAX_RECOMMENDATION_INGREDIENT_ROWS + 1)
    .fetch_all(&mut **transaction)
    .await?;
    if ingredient_rows.len() as i64 > MAX_RECOMMENDATION_INGREDIENT_ROWS {
        return Err(AppError::PayloadTooLarge(
            "recommendation ingredient row limit exceeded".into(),
        ));
    }
    let mut ingredients = std::collections::HashMap::<String, Vec<IngredientInput>>::new();
    for row in ingredient_rows {
        ingredients
            .entry(row.get("recipe_id"))
            .or_default()
            .push(IngredientInput {
                name: row.get("name"),
                quantity_note: row.get("quantity_note"),
                canonical_ingredient_id: row.get("canonical_ingredient_id"),
            });
    }
    let group_rows = sqlx::query(
        "WITH candidates AS (
           SELECT r.id FROM recipes r
           WHERE (? AND r.owner_id = ?)
              OR (? AND EXISTS(SELECT 1 FROM favorites f
                               WHERE f.user_id = ? AND f.recipe_id = r.id)
                      AND (r.owner_id = ? OR r.id IN (SELECT id FROM public_eligible_recipes)))
              OR (? AND r.id IN (SELECT id FROM public_eligible_recipes)
                      AND EXISTS(SELECT 1 FROM favorite_accounts fa
                                 WHERE fa.viewer_id = ? AND fa.author_id = r.owner_id))
              OR (? AND r.owner_id <> ? AND r.id IN (SELECT id FROM public_eligible_recipes))
           ORDER BY r.id
           LIMIT ?
         )
         SELECT g.recipe_id, g.external_id AS group_id, g.optional, g.note,
                g.sort_order AS group_sort, a.external_id AS alternative_id, a.name,
                a.quantity_note, a.canonical_ingredient_id,
                a.sort_order AS alternative_sort
         FROM recipe_ingredient_groups g
         JOIN candidates c ON c.id = g.recipe_id
         JOIN recipe_ingredient_alternatives a ON a.group_id = g.id
         ORDER BY g.recipe_id, g.sort_order, a.sort_order
         LIMIT ?",
    )
    .bind(settings.include_own)
    .bind(user_id)
    .bind(settings.include_favorite_recipes)
    .bind(user_id)
    .bind(user_id)
    .bind(settings.include_favorite_accounts)
    .bind(user_id)
    .bind(settings.include_other_public)
    .bind(user_id)
    .bind(MAX_RECOMMENDATION_CANDIDATES)
    .bind(MAX_RECOMMENDATION_INGREDIENT_ROWS + 1)
    .fetch_all(&mut **transaction)
    .await?;
    if group_rows.len() as i64 > MAX_RECOMMENDATION_INGREDIENT_ROWS {
        return Err(AppError::PayloadTooLarge(
            "recommendation ingredient row limit exceeded".into(),
        ));
    }
    let mut groups = std::collections::HashMap::<String, Vec<IngredientGroupInput>>::new();
    for row in group_rows {
        let recipe_id: String = row.get("recipe_id");
        let group_id: String = row.get("group_id");
        let recipe_groups = groups.entry(recipe_id).or_default();
        if recipe_groups
            .last()
            .is_none_or(|group| group.id != group_id)
        {
            recipe_groups.push(IngredientGroupInput {
                id: group_id,
                optional: row.get("optional"),
                note: row.get("note"),
                sort_order: row.get("group_sort"),
                alternatives: Vec::new(),
            });
        }
        recipe_groups
            .last_mut()
            .expect("group was inserted")
            .alternatives
            .push(IngredientAlternativeInput {
                id: row.get("alternative_id"),
                name: row.get("name"),
                quantity_note: row.get("quantity_note"),
                canonical_ingredient_id: row.get("canonical_ingredient_id"),
                sort_order: row.get("alternative_sort"),
            });
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let id: String = row.get("id");
            let owner_id: String = row.get("owner_id");
            let recipe_ingredients = ingredients.remove(&id).unwrap_or_default();
            let mut recipe_groups = groups.remove(&id).unwrap_or_default();
            if recipe_groups.is_empty() {
                recipe_groups = legacy_projection_groups(&id, &recipe_ingredients);
            }
            Recipe {
                ingredients: recipe_ingredients,
                ingredient_model: "groups-v1".into(),
                ingredient_groups: recipe_groups,
                id,
                is_owner: owner_id == user_id,
                owner_id,
                author_handle: row.get("public_handle"),
                author_nickname: row.get("public_nickname"),
                title: row.get("title"),
                description: String::new(),
                instructions: String::new(),
                visibility: row.get("visibility"),
                publication_intent: row.get("publication_intent"),
                moderation_state: row.get("moderation_state"),
                moderation_review_state: row.get("moderation_review_state"),
                moderation_version: row.get("moderation_version"),
                difficulty: row.get("difficulty"),
                estimated_calories: row.get("estimated_calories"),
                version: row.get("version"),
                is_favorite: row.get::<i64, _>("is_favorite") != 0,
                is_favorite_author: row.get::<i64, _>("is_favorite_author") != 0,
                source: None,
                cover_image: None,
            }
        })
        .collect())
}

async fn hydrate_recommendation_recipes(
    transaction: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    recommendations: &mut Vec<Recommendation>,
) -> AppResult<()> {
    if recommendations.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT r.id, r.description, r.instructions, r.source_url, r.source_title,
                r.source_author, r.source_license AS recipe_source_license,
                r.source_attribution AS recipe_source_attribution,
                r.source_warnings, r.source_confidence,
                img.id AS image_id, img.width, img.height, img.alt_text,
                img.source_url AS image_source_url, img.source_license,
                img.source_attribution, img.version AS image_version
         FROM recipes r
         LEFT JOIN recipe_images img ON img.recipe_id = r.id
         WHERE (r.owner_id = ",
    );
    query.push_bind(user_id);
    query.push(" OR r.id IN (SELECT id FROM public_eligible_recipes)) AND r.id IN (");
    {
        let mut separated = query.separated(", ");
        for recommendation in recommendations.iter() {
            separated.push_bind(&recommendation.recipe.id);
        }
    }
    query.push(")");
    let rows = query.build().fetch_all(&mut **transaction).await?;
    let mut by_id = rows
        .into_iter()
        .map(|row| {
            let id: String = row.get("id");
            (id, row)
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut missing = std::collections::HashSet::new();
    for recommendation in recommendations.iter_mut() {
        let Some(row) = by_id.remove(&recommendation.recipe.id) else {
            missing.insert(recommendation.recipe.id.clone());
            continue;
        };
        recommendation.recipe.description = row.get("description");
        recommendation.recipe.instructions = row.get("instructions");
        let source_url: Option<String> = row.get("source_url");
        let source_title: Option<String> = row.get("source_title");
        let source_author: Option<String> = row.get("source_author");
        let source_license: Option<String> = row.get("recipe_source_license");
        let source_attribution: Option<String> = row.get("recipe_source_attribution");
        recommendation.recipe.source = if source_url.is_some()
            || source_title.is_some()
            || source_author.is_some()
            || source_license.is_some()
            || source_attribution.is_some()
        {
            let warnings = row
                .get::<Option<String>, _>("source_warnings")
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(|error| {
                    AppError::Data(format!("invalid stored recipe source warnings: {error}"))
                })?
                .unwrap_or_default();
            Some(RecipeSource {
                url: source_url,
                title: source_title,
                author: source_author,
                license: source_license,
                attribution: source_attribution,
                warnings,
                confidence: row.get("source_confidence"),
            })
        } else {
            None
        };
        recommendation.recipe.cover_image =
            row.get::<Option<String>, _>("image_id")
                .map(|id| RecipeImage {
                    url: format!("/api/recipe-images/{id}"),
                    id,
                    width: row.get("width"),
                    height: row.get("height"),
                    alt_text: row.get("alt_text"),
                    source_url: row.get("image_source_url"),
                    source_license: row.get("source_license"),
                    source_attribution: row.get("source_attribution"),
                    version: row.get("image_version"),
                });
    }
    recommendations.retain(|recommendation| !missing.contains(&recommendation.recipe.id));
    Ok(())
}

async fn discard_unauthorized_recommendations(
    pool: &SqlitePool,
    user_id: &str,
    recommendations: &mut Vec<Recommendation>,
) -> AppResult<()> {
    if recommendations.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id FROM recipes WHERE (owner_id = ");
    query.push_bind(user_id);
    query.push(" OR id IN (SELECT id FROM public_eligible_recipes)) AND id IN (");
    {
        let mut separated = query.separated(", ");
        for recommendation in recommendations.iter() {
            separated.push_bind(&recommendation.recipe.id);
        }
    }
    query.push(")");
    let authorized = query
        .build_query_scalar::<String>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    recommendations.retain(|recommendation| authorized.contains(&recommendation.recipe.id));
    Ok(())
}

pub(crate) async fn load_ingredients(
    pool: &SqlitePool,
    recipe_id: &str,
) -> AppResult<Vec<IngredientInput>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, quantity_note, canonical_ingredient_id
         FROM recipe_ingredients WHERE recipe_id = ? ORDER BY sort_order",
    )
    .bind(recipe_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(name, quantity_note, canonical_ingredient_id)| IngredientInput {
                name,
                quantity_note,
                canonical_ingredient_id,
            },
        )
        .collect())
}

fn legacy_projection_groups(
    recipe_id: &str,
    ingredients: &[IngredientInput],
) -> Vec<IngredientGroupInput> {
    ingredients
        .iter()
        .enumerate()
        .map(|(index, ingredient)| IngredientGroupInput {
            id: format!("legacy:{recipe_id}:group:{index}"),
            optional: false,
            note: String::new(),
            sort_order: index as i64,
            alternatives: vec![IngredientAlternativeInput {
                id: format!("legacy:{recipe_id}:alternative:{index}"),
                name: ingredient.name.clone(),
                quantity_note: ingredient.quantity_note.clone(),
                canonical_ingredient_id: ingredient.canonical_ingredient_id.clone(),
                sort_order: 0,
            }],
        })
        .collect()
}

pub(crate) async fn load_ingredient_groups(
    pool: &SqlitePool,
    recipe_id: &str,
) -> AppResult<Vec<IngredientGroupInput>> {
    let rows = sqlx::query(
        "SELECT g.external_id AS group_id, g.optional, g.note, g.sort_order AS group_sort,
                a.external_id AS alternative_id, a.name, a.quantity_note,
                a.canonical_ingredient_id, a.sort_order AS alternative_sort
         FROM recipe_ingredient_groups g
         JOIN recipe_ingredient_alternatives a ON a.group_id = g.id
         WHERE g.recipe_id = ?
         ORDER BY g.sort_order, a.sort_order",
    )
    .bind(recipe_id)
    .fetch_all(pool)
    .await?;
    let mut groups = Vec::<IngredientGroupInput>::new();
    for row in rows {
        let group_id: String = row.get("group_id");
        if groups.last().is_none_or(|group| group.id != group_id) {
            groups.push(IngredientGroupInput {
                id: group_id,
                optional: row.get("optional"),
                note: row.get("note"),
                sort_order: row.get("group_sort"),
                alternatives: Vec::new(),
            });
        }
        groups
            .last_mut()
            .expect("group was inserted")
            .alternatives
            .push(IngredientAlternativeInput {
                id: row.get("alternative_id"),
                name: row.get("name"),
                quantity_note: row.get("quantity_note"),
                canonical_ingredient_id: row.get("canonical_ingredient_id"),
                sort_order: row.get("alternative_sort"),
            });
    }
    Ok(groups)
}

async fn ensure_visible_recipe(pool: &SqlitePool, user_id: &str, recipe_id: &str) -> AppResult<()> {
    let exists: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM recipes WHERE id = ? AND (
           owner_id = ? OR id IN (SELECT id FROM public_eligible_recipes)
         )",
    )
    .bind(recipe_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    exists.map(|_| ()).ok_or(AppError::NotFound)
}

type PantryRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
    i64,
);

async fn load_pantry(pool: &SqlitePool, user_id: &str) -> AppResult<Vec<PantryItem>> {
    let rows: Vec<PantryRow> = sqlx::query_as(
        "SELECT id, name, status, quantity_note, confirmed_at, expires_on,
                canonical_ingredient_id, auto_add_to_shopping_list_when_out, version
         FROM pantry_items WHERE owner_id = ? ORDER BY normalized_name LIMIT ?",
    )
    .bind(user_id)
    .bind(MAX_PANTRY_ITEMS_PER_OWNER + 1)
    .fetch_all(pool)
    .await?;
    let alias_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT canonical_ingredient_id, alias_normalized
         FROM ingredient_aliases WHERE owner_id = ?
         ORDER BY alias_normalized LIMIT ?",
    )
    .bind(user_id)
    .bind(MAX_INGREDIENT_ALIASES_PER_OWNER + 1)
    .fetch_all(pool)
    .await?;
    pantry_from_rows(rows, alias_rows)
}

async fn load_pantry_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    user_id: &str,
) -> AppResult<Vec<PantryItem>> {
    let rows: Vec<PantryRow> = sqlx::query_as(
        "SELECT id, name, status, quantity_note, confirmed_at, expires_on,
                canonical_ingredient_id, auto_add_to_shopping_list_when_out, version
         FROM pantry_items WHERE owner_id = ? ORDER BY normalized_name LIMIT ?",
    )
    .bind(user_id)
    .bind(MAX_PANTRY_ITEMS_PER_OWNER + 1)
    .fetch_all(&mut **transaction)
    .await?;
    let alias_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT canonical_ingredient_id, alias_normalized
         FROM ingredient_aliases WHERE owner_id = ?
         ORDER BY alias_normalized LIMIT ?",
    )
    .bind(user_id)
    .bind(MAX_INGREDIENT_ALIASES_PER_OWNER + 1)
    .fetch_all(&mut **transaction)
    .await?;
    pantry_from_rows(rows, alias_rows)
}

fn pantry_from_rows(
    rows: Vec<PantryRow>,
    alias_rows: Vec<(String, String)>,
) -> AppResult<Vec<PantryItem>> {
    if rows.len() as i64 > MAX_PANTRY_ITEMS_PER_OWNER {
        return Err(AppError::Validation("pantry item limit exceeded".into()));
    }
    if alias_rows.len() as i64 > MAX_INGREDIENT_ALIASES_PER_OWNER {
        return Err(AppError::Validation(
            "ingredient alias limit exceeded".into(),
        ));
    }
    let mut aliases = std::collections::HashMap::<String, Vec<String>>::new();
    for (canonical_id, normalized_alias) in alias_rows {
        aliases
            .entry(canonical_id)
            .or_default()
            .push(normalized_alias);
    }
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                name,
                status,
                quantity_note,
                confirmed_at,
                expires_on,
                canonical_ingredient_id,
                auto_add_to_shopping_list_when_out,
                version,
            )| {
                let item_aliases = canonical_ingredient_id
                    .as_ref()
                    .and_then(|id| aliases.get(id))
                    .cloned()
                    .unwrap_or_default();
                PantryItem {
                    id,
                    name,
                    status: PantryStatus::parse(&status),
                    quantity_note,
                    confirmed_at,
                    expires_on,
                    canonical_ingredient_id,
                    aliases: item_aliases,
                    auto_add_to_shopping_list_when_out,
                    version,
                }
            },
        )
        .collect())
}

async fn load_settings(pool: &SqlitePool, user_id: &str) -> AppResult<RecommendationSettings> {
    let row: Option<(bool, bool, bool, bool, i64)> = sqlx::query_as(
        "SELECT include_favorites, include_own, include_public,
                include_favorite_accounts, verification_days
         FROM recommendation_settings WHERE owner_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row
        .map(
            |(
                include_favorite_recipes,
                include_own,
                include_other_public,
                include_favorite_accounts,
                verification_days,
            )| {
                RecommendationSettings {
                    include_favorite_recipes,
                    include_own,
                    include_favorite_accounts,
                    include_other_public,
                    verification_days,
                }
            },
        )
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc, time::Instant};

    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request, StatusCode},
    };
    use chrono::Utc;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use serde_json::{json, Value};
    use sqlx::{Row, SqlitePool};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::assisted_session_request_allowed;
    use crate::{
        auth::{build_auth, AuthRuntime},
        build_app,
        config::{AppEnvironment, AuthMode, Config},
        db,
        passwordless::GOOGLE_IDENTITY_ISSUER,
        AppState,
    };

    #[test]
    fn assisted_session_write_policy_inventories_every_authenticated_write_route() {
        let inventory = [
            ("PUT", "/api/settings/profile", true),
            ("PUT", "/api/settings/locale", true),
            ("POST", "/api/auth/logout-all", false),
            ("POST", "/api/settings/security/email/request", false),
            ("POST", "/api/settings/security/email/verify", false),
            (
                "POST",
                "/api/settings/security/passkeys/register/start",
                false,
            ),
            (
                "POST",
                "/api/settings/security/passkeys/register/finish",
                false,
            ),
            ("DELETE", "/api/settings/security/passkeys/{id}", false),
            ("POST", "/api/recipes", true),
            ("PUT", "/api/recipes/{id}", true),
            ("DELETE", "/api/recipes/{id}", true),
            ("PUT", "/api/recipes/{id}/favorite", true),
            ("DELETE", "/api/recipes/{id}/favorite", true),
            ("PUT", "/api/authors/{handle}/favorite", true),
            ("DELETE", "/api/authors/{handle}/favorite", true),
            ("POST", "/api/recipes/{id}/cover", true),
            ("PUT", "/api/recipes/{id}/cover", true),
            ("DELETE", "/api/recipes/{id}/cover", true),
            ("POST", "/api/recipe-revisions/{id}/accept", true),
            ("DELETE", "/api/recipe-revisions/{id}", true),
            ("PUT", "/api/pantry", true),
            ("POST", "/api/shopping-list", true),
            ("PUT", "/api/shopping-list/{id}", true),
            ("DELETE", "/api/shopping-list/{id}", true),
            ("POST", "/api/shopping-list/{id}/purchase", true),
            ("POST", "/api/shopping-list/purchases/{id}/undo", true),
            ("POST", "/api/recipes/{id}/shopping-list-items", true),
            ("PUT", "/api/settings/recommendations", true),
            ("POST", "/api/recommendations", true),
            ("POST", "/api/settings/import-tokens", false),
            ("DELETE", "/api/settings/import-tokens/{id}", false),
            (
                "POST",
                "/api/capability-claims/content-moderator/passkey/start",
                false,
            ),
            (
                "POST",
                "/api/capability-claims/content-moderator/passkey/finish",
                false,
            ),
            (
                "POST",
                "/api/capability-claims/account-assistant/passkey/start",
                false,
            ),
            (
                "POST",
                "/api/capability-claims/account-assistant/passkey/finish",
                false,
            ),
            ("POST", "/api/recipes/{id}/reports", true),
            ("PUT", "/api/notifications/{id}/read", true),
            ("POST", "/api/recipes/{id}/appeals", true),
            ("POST", "/api/admin/reauth/passkey/start", false),
            ("POST", "/api/admin/reauth/passkey/finish", false),
            ("POST", "/api/admin/account-assistance/inspect", false),
            ("POST", "/api/admin/account-assistance/otp", false),
            ("POST", "/api/admin/moderation/recipes/{id}/review", false),
            ("POST", "/api/admin/moderation/recipes/{id}/restrict", false),
            (
                "POST",
                "/api/admin/moderation/recipes/{id}/reinstate",
                false,
            ),
            ("POST", "/api/admin/moderation/appeals/{id}/resolve", false),
            ("PUT", "/api/import-drafts/{id}", true),
            ("DELETE", "/api/import-drafts/{id}", true),
            ("POST", "/api/import-drafts/{id}/accept", true),
        ];
        for (method, path, expected) in inventory {
            let method = Method::from_bytes(method.as_bytes()).unwrap();
            assert_eq!(
                assisted_session_request_allowed(&method, Some(path)),
                expected,
                "{method} {path}"
            );
        }
        assert!(assisted_session_request_allowed(
            &Method::GET,
            Some("/api/recipes")
        ));
        assert!(!assisted_session_request_allowed(
            &Method::GET,
            Some("/api/settings/security")
        ));
        assert!(!assisted_session_request_allowed(
            &Method::GET,
            Some("/api/settings/import-tokens")
        ));
        assert!(!assisted_session_request_allowed(
            &Method::GET,
            Some("/api/admin/moderation/queue")
        ));
        assert!(!assisted_session_request_allowed(
            &Method::POST,
            Some("/api/future-account-recovery")
        ));
        assert!(!assisted_session_request_allowed(&Method::POST, None));
        assert!(!assisted_session_request_allowed(&Method::GET, None));
    }

    async fn app_with_runtime() -> (axum::Router, AuthRuntime, SqlitePool) {
        let config = Config::test();
        let pool = db::connect(&config.database_url).await.unwrap();
        let auth = build_auth(&config, &pool).unwrap();
        let app = super::router(AppState {
            pool: pool.clone(),
            config: Arc::new(config),
            auth: auth.clone(),
        });
        (app, auth, pool)
    }

    async fn login(app: &axum::Router, name: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/auth/dev-login")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "displayName": name }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    async fn create_recipe(app: &axum::Router, cookie: &str, visibility: &str) -> Value {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!({
                            "title": "Test soup",
                            "description": "",
                            "instructions": "",
                            "visibility": visibility,
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn grant_fresh_moderator(pool: &SqlitePool, display_name: &str) -> String {
        let user_id: String = sqlx::query_scalar("SELECT id FROM users WHERE display_name = ?")
            .bind(display_name)
            .fetch_one(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO capability_assignments (id, user_id, capability, created_at)
             VALUES (?, ?, 'content_moderation', ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&user_id)
        .bind(Utc::now().timestamp())
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("UPDATE auth_sessions SET passkey_verified_at = ? WHERE user_id = ?")
            .bind(Utc::now().timestamp())
            .bind(&user_id)
            .execute(pool)
            .await
            .unwrap();
        user_id
    }

    async fn report_recipe(app: &axum::Router, reporter_cookie: &str, recipe_id: &str) -> Value {
        let response = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/reports"))
                    .header("content-type", "application/json")
                    .header("cookie", reporter_cookie)
                    .body(Body::from(
                        json!({"reason": "misleading", "details": "incorrect claim"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        response_json(response).await
    }

    async fn admin_recipe_mutation(
        app: &axum::Router,
        cookie: &str,
        recipe_id: &str,
        action: &str,
        key: &str,
        body: &Value,
    ) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::post(format!(
                    "/api/admin/moderation/recipes/{recipe_id}/{action}"
                ))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .header("origin", "http://localhost")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn moderation_enforces_anonymity_step_up_idempotency_and_public_removal() {
        let (app, _auth, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Moderation owner").await;
        let reporter_cookie = login(&app, "Moderation reporter").await;
        let moderator_cookie = login(&app, "Moderation moderator").await;
        let recipe = create_recipe(&app, &owner_cookie, "public").await;
        let recipe_id = recipe["id"].as_str().unwrap();

        let self_report = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/reports"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({"reason": "spam", "details": ""}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(self_report.status(), StatusCode::NOT_FOUND);

        let report = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/reports"))
                    .header("content-type", "application/json")
                    .header("cookie", &reporter_cookie)
                    .body(Body::from(
                        json!({"reason": "misleading", "details": "<script>alert(1)</script>"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(report.status(), StatusCode::CREATED);

        let non_moderator_queue = app
            .clone()
            .oneshot(
                Request::get("/api/admin/moderation/queue")
                    .header("cookie", &reporter_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(non_moderator_queue.status(), StatusCode::NOT_FOUND);

        let moderator_id: String =
            sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'Moderation moderator'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let assignment_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO capability_assignments (id, user_id, capability, created_at)
             VALUES (?, ?, 'content_moderation', ?)",
        )
        .bind(&assignment_id)
        .bind(&moderator_id)
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();

        let key = Uuid::new_v4().to_string();
        let restriction_without_case_version = json!({
            "expectedRecipeVersion": recipe["version"],
            "expectedModerationVersion": recipe["moderationVersion"],
            "expectedCaseVersion": null,
            "reasonCode": "policy_violation"
        });
        let expired_step_up = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/admin/moderation/recipes/{recipe_id}/restrict"
                ))
                .header("content-type", "application/json")
                .header("cookie", &moderator_cookie)
                .header("origin", "http://localhost")
                .header("idempotency-key", &key)
                .body(Body::from(restriction_without_case_version.to_string()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(expired_step_up.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response_json(expired_step_up).await["error"],
            "admin_passkey_reauth_required"
        );

        sqlx::query("UPDATE auth_sessions SET passkey_verified_at = ? WHERE user_id = ?")
            .bind(Utc::now().timestamp())
            .bind(&moderator_id)
            .execute(&pool)
            .await
            .unwrap();
        let active_case_version: i64 = sqlx::query_scalar(
            "SELECT version FROM moderation_cases
                 WHERE recipe_id = ? AND status = 'open'",
        )
        .bind(recipe_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let missing_case_version = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/admin/moderation/recipes/{recipe_id}/restrict"
                ))
                .header("content-type", "application/json")
                .header("cookie", &moderator_cookie)
                .header("origin", "http://localhost")
                .header("idempotency-key", Uuid::new_v4().to_string())
                .body(Body::from(restriction_without_case_version.to_string()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_case_version.status(), StatusCode::CONFLICT);
        assert_eq!(
            response_json(missing_case_version).await["caseVersion"],
            active_case_version
        );
        let restriction = json!({
            "expectedRecipeVersion": recipe["version"],
            "expectedModerationVersion": recipe["moderationVersion"],
            "expectedCaseVersion": active_case_version,
            "reasonCode": "policy_violation"
        });
        sqlx::query(
            "INSERT INTO recipe_images
             (id, recipe_id, media_type, image_bytes, width, height, sha256, alt_text, updated_at)
             VALUES ('old-public-image', ?, 'image/jpeg', X'0102', 1, 1, zeroblob(32), 'cover', ?)",
        )
        .bind(recipe_id)
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();

        let restricted = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/admin/moderation/recipes/{recipe_id}/restrict"
                ))
                .header("content-type", "application/json")
                .header("cookie", &moderator_cookie)
                .header("origin", "http://localhost")
                .header("idempotency-key", &key)
                .body(Body::from(restriction.to_string()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(restricted.status(), StatusCode::OK);
        let restricted_body = response_json(restricted).await;

        let replayed = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/admin/moderation/recipes/{recipe_id}/restrict"
                ))
                .header("content-type", "application/json")
                .header("cookie", &moderator_cookie)
                .header("origin", "http://localhost")
                .header("idempotency-key", &key)
                .body(Body::from(restriction.to_string()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replayed.status(), StatusCode::OK);
        assert_eq!(response_json(replayed).await, restricted_body);

        let stale = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/admin/moderation/recipes/{recipe_id}/restrict"
                ))
                .header("content-type", "application/json")
                .header("cookie", &moderator_cookie)
                .header("origin", "http://localhost")
                .header("idempotency-key", Uuid::new_v4().to_string())
                .body(Body::from(restriction.to_string()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        let stale_body = response_json(stale).await;
        assert_eq!(stale_body["recipeVersion"], recipe["version"]);
        assert_eq!(
            stale_body["moderationVersion"],
            restricted_body["moderationVersion"]
        );

        let hidden_recipe = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &reporter_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hidden_recipe.status(), StatusCode::NOT_FOUND);
        let owner_recipe = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(owner_recipe.status(), StatusCode::OK);
        assert_eq!(
            response_json(owner_recipe).await["publicationIntent"],
            "public"
        );

        let old_image = app
            .clone()
            .oneshot(
                Request::get("/api/recipe-images/old-public-image")
                    .header("cookie", &reporter_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(old_image.status(), StatusCode::NOT_FOUND);

        let case_id = restricted_body["caseId"].as_str().unwrap();
        let case_detail = app
            .clone()
            .oneshot(
                Request::get(format!("/api/admin/moderation/cases/{case_id}"))
                    .header("cookie", &moderator_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(case_detail.status(), StatusCode::OK);
        let case_body = response_json(case_detail).await;
        assert_eq!(
            case_body["reports"][0]["details"],
            "<script>alert(1)</script>"
        );
        assert!(case_body["reports"][0].get("reporterUserId").is_none());
        assert!(case_body.get("assignmentId").is_none());

        let audit_id: String = sqlx::query_scalar("SELECT id FROM moderation_audit_events LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(sqlx::query(
            "UPDATE moderation_audit_events SET event_type = 'changed' WHERE id = ?"
        )
        .bind(&audit_id)
        .execute(&pool)
        .await
        .is_err());
        assert!(
            sqlx::query("DELETE FROM moderation_audit_events WHERE id = ?")
                .bind(audit_id)
                .execute(&pool)
                .await
                .is_err()
        );

        let deleted = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        let case_status: String =
            sqlx::query_scalar("SELECT status FROM moderation_cases WHERE id = ?")
                .bind(case_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(case_status, "content_removed");
    }

    #[tokio::test]
    async fn moderation_idempotency_is_resource_bound_and_review_closes_open_case() {
        let (app, _auth, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Resource owner").await;
        let reporter_cookie = login(&app, "Resource reporter").await;
        let moderator_cookie = login(&app, "Resource moderator").await;
        grant_fresh_moderator(&pool, "Resource moderator").await;

        let first = create_recipe(&app, &owner_cookie, "public").await;
        let second = create_recipe(&app, &owner_cookie, "public").await;
        let shared_key = Uuid::new_v4().to_string();
        let review_without_case = json!({
            "expectedRecipeVersion": 1,
            "expectedModerationVersion": 1,
            "expectedCaseVersion": null,
            "reasonCode": "no_action_needed"
        });
        let first_review = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            first["id"].as_str().unwrap(),
            "review",
            &shared_key,
            &review_without_case,
        )
        .await;
        assert_eq!(first_review.status(), StatusCode::OK);
        let cross_resource_reuse = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            second["id"].as_str().unwrap(),
            "review",
            &shared_key,
            &review_without_case,
        )
        .await;
        assert_eq!(cross_resource_reuse.status(), StatusCode::CONFLICT);

        let reported = create_recipe(&app, &owner_cookie, "public").await;
        let reported_id = reported["id"].as_str().unwrap();
        report_recipe(&app, &reporter_cookie, reported_id).await;
        let case_row = sqlx::query(
            "SELECT id, version FROM moderation_cases
             WHERE recipe_id = ? AND status = 'open'",
        )
        .bind(reported_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let review_report = json!({
            "expectedRecipeVersion": reported["version"],
            "expectedModerationVersion": reported["moderationVersion"],
            "expectedCaseVersion": case_row.get::<i64, _>("version"),
            "reasonCode": "report_resolved"
        });
        let reviewed = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            reported_id,
            "review",
            &Uuid::new_v4().to_string(),
            &review_report,
        )
        .await;
        assert_eq!(reviewed.status(), StatusCode::OK);
        let reviewed_body = response_json(reviewed).await;
        assert_eq!(reviewed_body["caseId"], case_row.get::<String, _>("id"));
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM moderation_cases WHERE id = ?")
                .bind(case_row.get::<String, _>("id"))
                .fetch_one(&pool)
                .await
                .unwrap(),
            "resolved"
        );
        let queue = app
            .clone()
            .oneshot(
                Request::get("/api/admin/moderation/queue")
                    .header("cookie", &moderator_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(queue.status(), StatusCode::OK);
        assert!(response_json(queue).await["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["recipeId"] != reported_id));
    }

    #[tokio::test]
    async fn restricted_edits_refresh_evidence_and_invalidate_stale_reinstate() {
        let (app, _auth, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Evidence owner").await;
        let reporter_cookie = login(&app, "Evidence reporter").await;
        let moderator_cookie = login(&app, "Evidence moderator").await;
        grant_fresh_moderator(&pool, "Evidence moderator").await;
        let recipe = create_recipe(&app, &owner_cookie, "public").await;
        let recipe_id = recipe["id"].as_str().unwrap();
        report_recipe(&app, &reporter_cookie, recipe_id).await;
        let initial_case_version: i64 = sqlx::query_scalar(
            "SELECT version FROM moderation_cases
             WHERE recipe_id = ? AND status = 'open'",
        )
        .bind(recipe_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let restrict_body = json!({
            "expectedRecipeVersion": recipe["version"],
            "expectedModerationVersion": recipe["moderationVersion"],
            "expectedCaseVersion": initial_case_version,
            "reasonCode": "policy_violation"
        });
        let restricted = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            recipe_id,
            "restrict",
            &Uuid::new_v4().to_string(),
            &restrict_body,
        )
        .await;
        assert_eq!(restricted.status(), StatusCode::OK);
        let restricted = response_json(restricted).await;

        let owner_update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(
                        json!({
                            "title": "Corrected restricted soup",
                            "description": "current evidence",
                            "instructions": "Use the corrected method.",
                            "visibility": "public",
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(owner_update.status(), StatusCode::OK);
        let owner_update = response_json(owner_update).await;
        let evidence = sqlx::query(
            "SELECT e.recipe_version, e.title, c.version AS case_version
             FROM moderation_evidence_snapshots e
             JOIN moderation_cases c ON c.id = e.case_id
             WHERE e.recipe_id = ? AND c.status = 'restricted'",
        )
        .bind(recipe_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            evidence.get::<i64, _>("recipe_version"),
            owner_update["version"]
        );
        assert_eq!(
            evidence.get::<String, _>("title"),
            "Corrected restricted soup"
        );
        assert!(
            evidence.get::<i64, _>("case_version") > restricted["caseVersion"].as_i64().unwrap()
        );

        let stale_reinstate = json!({
            "expectedRecipeVersion": restricted["recipeVersion"],
            "expectedModerationVersion": restricted["moderationVersion"],
            "expectedCaseVersion": restricted["caseVersion"],
            "reasonCode": "appeal_reviewed"
        });
        let stale = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            recipe_id,
            "reinstate",
            &Uuid::new_v4().to_string(),
            &stale_reinstate,
        )
        .await;
        assert_eq!(stale.status(), StatusCode::CONFLICT);

        let appeal = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/appeals"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({"reason": "Please review the corrected version."}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(appeal.status(), StatusCode::CREATED);
        let appeal = response_json(appeal).await;
        let current_reinstate = json!({
            "resolution": "reinstate",
            "expectedRecipeVersion": owner_update["version"],
            "expectedModerationVersion": owner_update["moderationVersion"],
            "expectedCaseVersion": evidence.get::<i64, _>("case_version"),
            "expectedAppealVersion": appeal["version"],
            "reasonCode": "appeal_reviewed"
        });
        let reinstated = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/admin/moderation/appeals/{}/resolve",
                    appeal["id"].as_str().unwrap()
                ))
                .header("content-type", "application/json")
                .header("cookie", &moderator_cookie)
                .header("origin", "http://localhost")
                .header("idempotency-key", Uuid::new_v4().to_string())
                .body(Body::from(current_reinstate.to_string()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reinstated.status(), StatusCode::OK);
        let public_recipe = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &reporter_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(public_recipe.status(), StatusCode::OK);
        assert_eq!(
            response_json(public_recipe).await["title"],
            "Corrected restricted soup"
        );
    }

    #[tokio::test]
    async fn open_case_edits_refresh_evidence_before_review_or_restriction() {
        let (app, _auth, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Open evidence owner").await;
        let reporter_cookie = login(&app, "Open evidence reporter").await;
        let moderator_cookie = login(&app, "Open evidence moderator").await;
        grant_fresh_moderator(&pool, "Open evidence moderator").await;

        let review_recipe = create_recipe(&app, &owner_cookie, "public").await;
        let review_recipe_id = review_recipe["id"].as_str().unwrap();
        report_recipe(&app, &reporter_cookie, review_recipe_id).await;
        let stale_case_version: i64 = sqlx::query_scalar(
            "SELECT version FROM moderation_cases
             WHERE recipe_id = ? AND status = 'open'",
        )
        .bind(review_recipe_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let review_update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{review_recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(
                        json!({
                            "title": "Open case v2",
                            "description": "current open evidence",
                            "instructions": "Review this version.",
                            "visibility": "public",
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(review_update.status(), StatusCode::OK);
        let review_update = response_json(review_update).await;
        let review_evidence = sqlx::query(
            "SELECT e.recipe_version, e.title, c.version AS case_version
             FROM moderation_evidence_snapshots e
             JOIN moderation_cases c ON c.id = e.case_id
             WHERE e.recipe_id = ? AND c.status = 'open'",
        )
        .bind(review_recipe_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            review_evidence.get::<i64, _>("recipe_version"),
            review_update["version"]
        );
        assert_eq!(review_evidence.get::<String, _>("title"), "Open case v2");
        assert!(review_evidence.get::<i64, _>("case_version") > stale_case_version);
        let stale_review = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            review_recipe_id,
            "review",
            &Uuid::new_v4().to_string(),
            &json!({
                "expectedRecipeVersion": review_update["version"],
                "expectedModerationVersion": review_update["moderationVersion"],
                "expectedCaseVersion": stale_case_version,
                "reasonCode": "report_resolved"
            }),
        )
        .await;
        assert_eq!(stale_review.status(), StatusCode::CONFLICT);
        let reviewed = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            review_recipe_id,
            "review",
            &Uuid::new_v4().to_string(),
            &json!({
                "expectedRecipeVersion": review_update["version"],
                "expectedModerationVersion": review_update["moderationVersion"],
                "expectedCaseVersion": review_evidence.get::<i64, _>("case_version"),
                "reasonCode": "report_resolved"
            }),
        )
        .await;
        assert_eq!(reviewed.status(), StatusCode::OK);

        let restrict_recipe = create_recipe(&app, &owner_cookie, "public").await;
        let restrict_recipe_id = restrict_recipe["id"].as_str().unwrap();
        report_recipe(&app, &reporter_cookie, restrict_recipe_id).await;
        let restrict_update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{restrict_recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(
                        json!({
                            "title": "Restrict current v2",
                            "description": "current restriction evidence",
                            "instructions": "Restrict this version.",
                            "visibility": "public",
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(restrict_update.status(), StatusCode::OK);
        let restrict_update = response_json(restrict_update).await;
        let current_case_version: i64 = sqlx::query_scalar(
            "SELECT version FROM moderation_cases
             WHERE recipe_id = ? AND status = 'open'",
        )
        .bind(restrict_recipe_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let restricted = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            restrict_recipe_id,
            "restrict",
            &Uuid::new_v4().to_string(),
            &json!({
                "expectedRecipeVersion": restrict_update["version"],
                "expectedModerationVersion": restrict_update["moderationVersion"],
                "expectedCaseVersion": current_case_version,
                "reasonCode": "policy_violation"
            }),
        )
        .await;
        assert_eq!(restricted.status(), StatusCode::OK);
        let restricted = response_json(restricted).await;
        let reinstated = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            restrict_recipe_id,
            "reinstate",
            &Uuid::new_v4().to_string(),
            &json!({
                "expectedRecipeVersion": restricted["recipeVersion"],
                "expectedModerationVersion": restricted["moderationVersion"],
                "expectedCaseVersion": restricted["caseVersion"],
                "reasonCode": "appeal_reviewed"
            }),
        )
        .await;
        assert_eq!(reinstated.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn moderation_evidence_image_requires_fresh_non_owner_case_access() {
        let (app, _auth, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Image evidence owner").await;
        let reporter_cookie = login(&app, "Image evidence reporter").await;
        let moderator_cookie = login(&app, "Image evidence moderator").await;
        let moderator_id = grant_fresh_moderator(&pool, "Image evidence moderator").await;
        let recipe = create_recipe(&app, &owner_cookie, "public").await;
        let recipe_id = recipe["id"].as_str().unwrap();
        sqlx::query(
            "INSERT INTO recipe_images
             (id, recipe_id, media_type, image_bytes, width, height, sha256, alt_text, updated_at)
             VALUES ('evidence-public-image', ?, 'image/jpeg', X'0102', 1, 1,
                     zeroblob(32), 'evidence cover', ?)",
        )
        .bind(recipe_id)
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();
        report_recipe(&app, &reporter_cookie, recipe_id).await;
        let case_version: i64 = sqlx::query_scalar(
            "SELECT version FROM moderation_cases
             WHERE recipe_id = ? AND status = 'open'",
        )
        .bind(recipe_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let restricted = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            recipe_id,
            "restrict",
            &Uuid::new_v4().to_string(),
            &json!({
                "expectedRecipeVersion": recipe["version"],
                "expectedModerationVersion": recipe["moderationVersion"],
                "expectedCaseVersion": case_version,
                "reasonCode": "policy_violation"
            }),
        )
        .await;
        assert_eq!(restricted.status(), StatusCode::OK);
        let restricted = response_json(restricted).await;
        let case_id = restricted["caseId"].as_str().unwrap();
        let private_image_id: String = sqlx::query_scalar(
            "SELECT image_id FROM moderation_evidence_snapshots WHERE case_id = ?",
        )
        .bind(case_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let ordinary_image = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipe-images/{private_image_id}"))
                    .header("cookie", &reporter_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ordinary_image.status(), StatusCode::NOT_FOUND);

        let evidence_image = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/admin/moderation/cases/{case_id}/evidence-image"
                ))
                .header("cookie", &moderator_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(evidence_image.status(), StatusCode::OK);
        assert_eq!(
            evidence_image.headers()["cache-control"],
            "private, no-store"
        );
        assert_eq!(
            to_bytes(evidence_image.into_body(), usize::MAX)
                .await
                .unwrap(),
            &[1, 2][..]
        );
        let reinstated = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            recipe_id,
            "reinstate",
            &Uuid::new_v4().to_string(),
            &json!({
                "expectedRecipeVersion": restricted["recipeVersion"],
                "expectedModerationVersion": restricted["moderationVersion"],
                "expectedCaseVersion": restricted["caseVersion"],
                "reasonCode": "appeal_reviewed"
            }),
        )
        .await;
        assert_eq!(reinstated.status(), StatusCode::OK);
        sqlx::query(
            "UPDATE recipe_images
             SET id = 'replacement-image', image_bytes = X'0304', sha256 = zeroblob(32)
             WHERE recipe_id = ?",
        )
        .bind(recipe_id)
        .execute(&pool)
        .await
        .unwrap();
        let after_replacement = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/admin/moderation/cases/{case_id}/evidence-image"
                ))
                .header("cookie", &moderator_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(after_replacement.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(after_replacement.into_body(), usize::MAX)
                .await
                .unwrap(),
            &[1, 2][..]
        );
        let deleted_cover = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}/cover"))
                    .header("cookie", &owner_cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .header("idempotency-key", cover_key("evidence-delete"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted_cover.status(), StatusCode::OK);
        let retained_evidence = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/admin/moderation/cases/{case_id}/evidence-image"
                ))
                .header("cookie", &moderator_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retained_evidence.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(retained_evidence.into_body(), usize::MAX)
                .await
                .unwrap(),
            &[1, 2][..]
        );
        let retained_row = sqlx::query(
            "SELECT length(image_bytes) AS byte_count, length(image_sha256) AS digest_size
             FROM moderation_evidence_snapshots WHERE case_id = ?",
        )
        .bind(case_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(retained_row.get::<i64, _>("byte_count"), 2);
        assert_eq!(retained_row.get::<i64, _>("digest_size"), 32);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM recipe_images WHERE recipe_id = ?",)
                .bind(recipe_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );

        let non_moderator = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/admin/moderation/cases/{case_id}/evidence-image"
                ))
                .header("cookie", &reporter_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(non_moderator.status(), StatusCode::NOT_FOUND);
        sqlx::query("UPDATE auth_sessions SET passkey_verified_at = ? WHERE user_id = ?")
            .bind(Utc::now().timestamp() - 11 * 60)
            .bind(&moderator_id)
            .execute(&pool)
            .await
            .unwrap();
        let stale_moderator = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/admin/moderation/cases/{case_id}/evidence-image"
                ))
                .header("cookie", &moderator_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale_moderator.status(), StatusCode::FORBIDDEN);

        grant_fresh_moderator(&pool, "Image evidence owner").await;
        let self_moderation = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/admin/moderation/cases/{case_id}/evidence-image"
                ))
                .header("cookie", &owner_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(self_moderation.status(), StatusCode::NOT_FOUND);
        sqlx::query("DELETE FROM moderation_cases WHERE id = ?")
            .bind(case_id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM moderation_evidence_snapshots WHERE case_id = ?",
            )
            .bind(case_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn recipe_delete_preserves_active_case_image_evidence_and_rolls_back_atomically() {
        let (app, _auth, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Delete evidence owner").await;
        let reporter_cookie = login(&app, "Delete evidence reporter").await;
        let moderator_cookie = login(&app, "Delete evidence moderator").await;
        grant_fresh_moderator(&pool, "Delete evidence moderator").await;
        let recipe = create_recipe(&app, &owner_cookie, "public").await;
        let recipe_id = recipe["id"].as_str().unwrap();
        sqlx::query(
            "INSERT INTO recipe_images
             (id, recipe_id, media_type, image_bytes, width, height, sha256, alt_text, updated_at)
             VALUES ('delete-evidence-image', ?, 'image/jpeg', X'0102', 1, 1,
                     zeroblob(32), 'delete evidence cover', ?)",
        )
        .bind(recipe_id)
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();
        report_recipe(&app, &reporter_cookie, recipe_id).await;
        let case_version: i64 = sqlx::query_scalar(
            "SELECT version FROM moderation_cases
             WHERE recipe_id = ? AND status = 'open'",
        )
        .bind(recipe_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let restricted = admin_recipe_mutation(
            &app,
            &moderator_cookie,
            recipe_id,
            "restrict",
            &Uuid::new_v4().to_string(),
            &json!({
                "expectedRecipeVersion": recipe["version"],
                "expectedModerationVersion": recipe["moderationVersion"],
                "expectedCaseVersion": case_version,
                "reasonCode": "policy_violation"
            }),
        )
        .await;
        assert_eq!(restricted.status(), StatusCode::OK);
        let restricted = response_json(restricted).await;
        let case_id = restricted["caseId"].as_str().unwrap();
        let appeal = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/appeals"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({"reason": "Preserve this evidence during deletion."}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(appeal.status(), StatusCode::CREATED);
        let appeal_id = response_json(appeal).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let before = sqlx::query(
            "SELECT title, image_id, image_bytes, image_media_type, image_sha256
             FROM moderation_evidence_snapshots WHERE case_id = ?",
        )
        .bind(case_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let evidence_image_id: String = before.get("image_id");
        let evidence_bytes: Vec<u8> = before.get("image_bytes");
        let evidence_media_type: String = before.get("image_media_type");
        let evidence_digest: Vec<u8> = before.get("image_sha256");

        sqlx::query(
            "CREATE TRIGGER fail_recipe_delete_for_evidence
             BEFORE DELETE ON recipes
             BEGIN
               SELECT RAISE(ABORT, 'forced recipe delete failure');
             END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let failed_delete = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(failed_delete.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM moderation_cases WHERE id = ?")
                .bind(case_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "restricted"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM moderation_appeals WHERE id = ?")
                .bind(&appeal_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "active"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM recipes WHERE id = ?")
                .bind(recipe_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        sqlx::query("DROP TRIGGER fail_recipe_delete_for_evidence")
            .execute(&pool)
            .await
            .unwrap();

        let deleted = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM moderation_cases WHERE id = ?")
                .bind(case_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "content_removed"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM moderation_appeals WHERE id = ?")
                .bind(&appeal_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            "content_removed"
        );
        let after = sqlx::query(
            "SELECT title, image_id, image_bytes, image_media_type, image_sha256
             FROM moderation_evidence_snapshots WHERE case_id = ?",
        )
        .bind(case_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            after.get::<String, _>("title"),
            before.get::<String, _>("title")
        );
        assert_eq!(after.get::<String, _>("image_id"), evidence_image_id);
        assert_eq!(after.get::<Vec<u8>, _>("image_bytes"), evidence_bytes);
        assert_eq!(
            after.get::<String, _>("image_media_type"),
            evidence_media_type
        );
        assert_eq!(after.get::<Vec<u8>, _>("image_sha256"), evidence_digest);
        let evidence_image = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/admin/moderation/cases/{case_id}/evidence-image"
                ))
                .header("cookie", &moderator_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(evidence_image.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(evidence_image.into_body(), usize::MAX)
                .await
                .unwrap(),
            &[1, 2][..]
        );
        let ordinary_image = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipe-images/{evidence_image_id}"))
                    .header("cookie", &reporter_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ordinary_image.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn deleted_recipe_case_still_rejects_self_moderation() {
        let (app, _auth, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Deleted case owner").await;
        let reporter_cookie = login(&app, "Deleted case reporter").await;
        let recipe = create_recipe(&app, &owner_cookie, "public").await;
        let recipe_id = recipe["id"].as_str().unwrap();
        report_recipe(&app, &reporter_cookie, recipe_id).await;
        let case_id: String =
            sqlx::query_scalar("SELECT id FROM moderation_cases WHERE recipe_id = ?")
                .bind(recipe_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        grant_fresh_moderator(&pool, "Deleted case owner").await;
        let deleted = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        let self_case = app
            .clone()
            .oneshot(
                Request::get(format!("/api/admin/moderation/cases/{case_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(self_case.status(), StatusCode::NOT_FOUND);
    }

    async fn save_profile(app: &axum::Router, cookie: &str, nickname: Option<&str>) -> Value {
        let response = app
            .clone()
            .oneshot(
                Request::put("/api/settings/profile")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(json!({"nickname": nickname}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response_json(response).await
    }

    async fn delete_shopping_item(
        app: &axum::Router,
        cookie: &str,
        item_id: &str,
        expected_version: i64,
        key: &str,
    ) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::delete(format!("/api/shopping-list/{item_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .header("idempotency-key", key)
                    .body(Body::from(
                        json!({"expectedVersion": expected_version}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn save_locale(app: &axum::Router, cookie: &str, locale: &str) -> Value {
        let response = app
            .clone()
            .oneshot(
                Request::put("/api/settings/locale")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(json!({"locale": locale}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response_json(response).await
    }

    async fn create_token(app: &axum::Router, cookie: &str, preset: &str) -> Value {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/settings/import-tokens")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!({
                            "name": "Test client",
                            "expiresInDays": 90,
                            "preset": preset,
                            "confirmPublishRisk": preset == "publish"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        response_json(response).await
    }

    async fn create_import_token(app: &axum::Router, cookie: &str) -> Value {
        create_token(app, cookie, "import").await
    }

    async fn create_manage_token(app: &axum::Router, cookie: &str) -> Value {
        create_token(app, cookie, "manage").await
    }

    async fn create_publish_token(app: &axum::Router, cookie: &str) -> Value {
        create_token(app, cookie, "publish").await
    }

    fn import_payload(source_url: &str) -> Value {
        json!({
            "schemaVersion": "1.0",
            "sourceUrl": source_url,
            "sourceTitle": "Example soup",
            "sourceAuthor": "Example author",
            "sourceLicense": "CC-BY-4.0",
            "sourceAttribution": "Example author — Example soup",
            "title": "Imported soup",
            "description": "Review me",
            "difficulty": 2,
            "estimatedCalories": 450,
            "ingredients": [{"name": "water", "quantityNote": "1 cup"}],
            "steps": ["Stir the ingredients."],
            "warnings": ["Timing was inferred."],
            "confidence": 0.8
        })
    }

    async fn submit_import(
        app: &axum::Router,
        secret: &str,
        payload: Value,
    ) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::post("/api/import/drafts")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    fn test_png() -> Vec<u8> {
        let image = RgbaImage::from_pixel(2, 2, Rgba([40, 120, 200, 255]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn multipart_image(bytes: &[u8]) -> (String, Vec<u8>) {
        multipart_image_with_alt(bytes, "Blue soup")
    }

    fn multipart_image_with_alt(bytes: &[u8], alt_text: &str) -> (String, Vec<u8>) {
        let boundary = "cook-test-boundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"altText\"\r\n\r\n{alt_text}\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"cover.png\"\r\nContent-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    fn cover_key(label: &str) -> String {
        format!("{}.{}", Utc::now().timestamp(), label)
    }

    async fn set_cover_rate_count(pool: &SqlitePool, owner_id: &str, count: i64) {
        let now = Utc::now().timestamp();
        let window_started_at = now - now.rem_euclid(60 * 60);
        sqlx::query(
            "INSERT INTO public_api_rate_limits (rate_key, window_started_at, request_count)
             VALUES (?, ?, ?)
             ON CONFLICT(rate_key) DO UPDATE SET
               window_started_at = excluded.window_started_at,
               request_count = excluded.request_count",
        )
        .bind(format!("recipe-cover:{owner_id}"))
        .bind(window_started_at)
        .bind(count)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn private_recipe_is_not_visible_to_another_account() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let other_cookie = login(&app, "Other").await;
        let created = create_recipe(&app, &owner_cookie, "private").await;
        let response = app
            .oneshot(
                Request::get(format!("/api/recipes/{}", created["id"].as_str().unwrap()))
                    .header("cookie", other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn public_nickname_is_opt_in_and_does_not_expose_auth_profile() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Google Display Name").await;
        let other_cookie = login(&app, "Reader").await;

        let me = app
            .clone()
            .oneshot(
                Request::get("/api/me")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let me = response_json(me).await;
        assert_eq!(me["displayName"], "Google Display Name");
        assert_eq!(me["nickname"], Value::Null);
        assert_eq!(me["onboardingCompleted"], false);

        let created = create_recipe(&app, &owner_cookie, "public").await;
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipes/{}", created["id"].as_str().unwrap()))
                    .header("cookie", &other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let public_recipe = response_json(response).await;
        assert_eq!(public_recipe["authorNickname"], Value::Null);
        assert_eq!(public_recipe["isOwner"], false);
        assert!(public_recipe.get("ownerId").is_none());
        assert!(public_recipe.get("email").is_none());
        assert!(public_recipe.get("displayName").is_none());

        let profile = save_profile(&app, &owner_cookie, Some("厨师 🌶️")).await;
        assert_eq!(profile["nickname"], "厨师 🌶️");
        let response = app
            .oneshot(
                Request::get(format!("/api/recipes/{}", created["id"].as_str().unwrap()))
                    .header("cookie", other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let public_recipe = response_json(response).await;
        assert_eq!(public_recipe["authorNickname"], "厨师 🌶️");
    }

    #[tokio::test]
    async fn public_author_profiles_and_favorites_preserve_privacy_and_account_isolation() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Private auth name").await;
        let viewer_cookie = login(&app, "Viewer").await;
        let other_cookie = login(&app, "Other viewer").await;
        save_profile(&app, &owner_cookie, Some("Public cook")).await;
        let private_recipe = create_recipe(&app, &owner_cookie, "private").await;
        let public_recipe = create_recipe(&app, &owner_cookie, "public").await;
        let handle = public_recipe["authorHandle"].as_str().unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/authors/{handle}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let profile = response_json(response).await;
        assert_eq!(profile["handle"], handle);
        assert_eq!(profile["nickname"], "Public cook");
        assert_eq!(profile["recipes"].as_array().unwrap().len(), 1);
        assert_eq!(profile["recipes"][0]["id"], public_recipe["id"]);
        let serialized = profile.to_string();
        assert!(!serialized.contains("Private auth name"));
        assert!(!serialized.contains(private_recipe["id"].as_str().unwrap()));
        for forbidden in [
            "ownerId",
            "email",
            "displayName",
            "google",
            "subject",
            "pantry",
            "createdAt",
        ] {
            assert!(!profile.as_object().unwrap().contains_key(forbidden));
        }

        let self_favorite = app
            .clone()
            .oneshot(
                Request::put(format!("/api/authors/{handle}/favorite"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(self_favorite.status(), StatusCode::BAD_REQUEST);

        let favorite = app
            .clone()
            .oneshot(
                Request::put(format!("/api/authors/{handle}/favorite"))
                    .header("cookie", &viewer_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(favorite.status(), StatusCode::NO_CONTENT);
        let viewer_favorites = response_json(
            app.clone()
                .oneshot(
                    Request::get("/api/favorite-accounts")
                        .header("cookie", &viewer_cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(viewer_favorites.as_array().unwrap().len(), 1);
        assert_eq!(viewer_favorites[0]["handle"], handle);

        let other_favorites = response_json(
            app.oneshot(
                Request::get("/api/favorite-accounts")
                    .header("cookie", other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
        )
        .await;
        assert!(other_favorites.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn favorite_accounts_dynamically_supply_current_public_recipes() {
        let (app, _, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Owner").await;
        let viewer_cookie = login(&app, "Viewer").await;
        let first = create_recipe(&app, &owner_cookie, "public").await;
        let handle = first["authorHandle"].as_str().unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::put(format!("/api/authors/{handle}/favorite"))
                    .header("cookie", &viewer_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let second = create_recipe(&app, &owner_cookie, "public").await;

        let recommendations = response_json(
            app.clone()
                .oneshot(
                    Request::post("/api/recommendations")
                        .header("content-type", "application/json")
                        .header("cookie", &viewer_cookie)
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        let results = recommendations.as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| {
            result["matchedSources"]
                .as_array()
                .unwrap()
                .contains(&json!("FAVORITE_ACCOUNT"))
        }));

        sqlx::query(
            "UPDATE recipes
             SET visibility = 'private', publication_intent = 'private'
             WHERE id = ?",
        )
        .bind(second["id"].as_str().unwrap())
        .execute(&pool)
        .await
        .unwrap();
        let recommendations = response_json(
            app.oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &viewer_cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(recommendations.as_array().unwrap().len(), 1);
        assert_eq!(recommendations[0]["recipe"]["id"], first["id"]);
    }

    #[tokio::test]
    async fn recommendation_source_overrides_are_request_scoped() {
        let (app, _, _) = app_with_runtime().await;
        let owner_cookie = login(&app, "Owner").await;
        let viewer_cookie = login(&app, "Viewer").await;
        create_recipe(&app, &owner_cookie, "public").await;

        let hidden = response_json(
            app.clone()
                .oneshot(
                    Request::post("/api/recommendations")
                        .header("content-type", "application/json")
                        .header("cookie", &viewer_cookie)
                        .body(Body::from(
                            json!({
                                "includeFavoriteRecipes": false,
                                "includeOwn": false,
                                "includeFavoriteAccounts": false,
                                "includeOtherPublic": false
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert!(hidden.as_array().unwrap().is_empty());

        let defaults = response_json(
            app.oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", viewer_cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(defaults.as_array().unwrap().len(), 1);
        assert_eq!(defaults[0]["appliedControls"]["includeOtherPublic"], true);
    }

    #[tokio::test]
    async fn private_canonical_recommendation_filter_fails_closed_for_another_account() {
        let (app, _, _) = app_with_runtime().await;
        let owner_cookie = login(&app, "Owner").await;
        let viewer_cookie = login(&app, "Viewer").await;
        let private_recipe = create_recipe(&app, &owner_cookie, "private").await;
        let canonical_id = private_recipe["ingredients"][0]["canonicalIngredientId"]
            .as_str()
            .unwrap();
        let display_name = private_recipe["ingredients"][0]["name"].as_str().unwrap();

        let unavailable = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &viewer_cookie)
                    .body(Body::from(
                        json!({
                            "preferred": [{
                                "name": display_name,
                                "canonicalIngredientId": canonical_id
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unavailable.status(), StatusCode::BAD_REQUEST);
        let unavailable = response_json(unavailable).await;
        assert_eq!(unavailable["error"], "canonical_ingredient_unavailable");
        assert_eq!(unavailable["canonicalIngredientIds"], json!([canonical_id]));

        let unresolved = app
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", viewer_cookie)
                    .body(Body::from(
                        json!({
                            "preferred": [{
                                "name": display_name,
                                "canonicalIngredientId": null
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unresolved.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn recommendation_discards_recipe_unpublished_after_selection() {
        let path = std::env::temp_dir().join(format!(
            "cook-recommendation-snapshot-{}.db",
            Uuid::new_v4()
        ));
        let mut config = Config::test();
        config.database_url = format!("sqlite://{}", path.display());
        let app = build_app(config.clone()).await.unwrap();
        let pool = crate::db::connect(&config.database_url).await.unwrap();
        let owner_cookie = login(&app, "Snapshot owner").await;
        let viewer_cookie = login(&app, "Snapshot viewer").await;
        let recipe = create_recipe(&app, &owner_cookie, "public").await;
        let recipe_id = recipe["id"].as_str().unwrap().to_string();
        sqlx::query(
            "UPDATE recipes
             SET description = 'private-after-selection',
                 instructions = 'do-not-return',
                 source_url = 'https://private.example.test/source'
             WHERE id = ?",
        )
        .bind(&recipe_id)
        .execute(&pool)
        .await
        .unwrap();
        let viewer_id: String =
            sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'Snapshot viewer'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let hook = super::install_recommendation_selection_hook(viewer_id);
        let request_app = app.clone();
        let request = tokio::spawn(async move {
            request_app
                .oneshot(
                    Request::post("/api/recommendations")
                        .header("content-type", "application/json")
                        .header("cookie", &viewer_cookie)
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap()
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), hook.selected.notified())
            .await
            .expect("recommendation selection did not reach test hook");
        sqlx::query(
            "UPDATE recipes
             SET visibility = 'private', publication_intent = 'private'
             WHERE id = ?",
        )
        .bind(&recipe_id)
        .execute(&pool)
        .await
        .unwrap();
        hook.resume.notify_one();
        let response = request.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert!(body.as_array().unwrap().is_empty());
        let serialized = body.to_string();
        assert!(!serialized.contains("private-after-selection"));
        assert!(!serialized.contains("do-not-return"));
        assert!(!serialized.contains("private.example.test"));
    }

    #[tokio::test]
    async fn recommendation_scale_response_is_bounded_for_ten_thousand_candidates() {
        let (app, _, pool) = app_with_runtime().await;
        let viewer_cookie = login(&app, "Scale viewer").await;
        let viewer_id: String =
            sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'Scale viewer'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let mut transaction = pool.begin().await.unwrap();
        sqlx::query(
            "WITH RECURSIVE seq(x) AS (
               SELECT 1 UNION ALL SELECT x + 1 FROM seq WHERE x < 500
             )
             INSERT INTO users (id, email, display_name)
             SELECT printf('scale-author-%04d', x),
                    printf('scale-author-%04d@example.test', x),
                    'Scale author'
             FROM seq",
        )
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO favorite_accounts (viewer_id, author_id, created_at)
             SELECT ?, id, 0 FROM users WHERE id LIKE 'scale-author-%'",
        )
        .bind(&viewer_id)
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query(
            "WITH RECURSIVE seq(x) AS (
               SELECT 1 UNION ALL SELECT x + 1 FROM seq WHERE x < 10000
             )
             INSERT INTO recipes
               (id, owner_id, title, description, instructions, visibility,
                publication_intent, created_at, updated_at)
             SELECT printf('scale-recipe-%05d', x),
                    printf('scale-author-%04d', ((x - 1) % 500) + 1),
                    printf('Scale recipe %05d', x), '', '', 'public', 'public',
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
             FROM seq",
        )
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO recipe_ingredients
               (id, recipe_id, name, normalized_name, quantity_note, sort_order)
             SELECT 'ingredient-' || id, id, 'water', 'water', '', 0
             FROM recipes WHERE id LIKE 'scale-recipe-%'",
        )
        .execute(&mut *transaction)
        .await
        .unwrap();
        transaction.commit().await.unwrap();

        let started = Instant::now();
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations?limit=37")
                    .header("content-type", "application/json")
                    .header("cookie", &viewer_cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let elapsed = started.elapsed();
        assert_eq!(response.status(), StatusCode::OK);
        let recommendations = response_json(response).await;
        assert_eq!(recommendations.as_array().unwrap().len(), 37);
        eprintln!("10k candidate recommendation duration: {elapsed:?}");

        sqlx::query(
            "INSERT INTO recipes
               (id, owner_id, title, description, instructions, visibility,
                publication_intent, created_at, updated_at)
             VALUES ('scale-recipe-over-limit', 'scale-author-0001', 'Over limit',
                     '', '', 'public', 'public', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let over_limit = app
            .oneshot(
                Request::post("/api/recommendations?limit=37")
                    .header("content-type", "application/json")
                    .header("cookie", viewer_cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(over_limit.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn recommendation_marks_active_shopping_membership_by_owner_identity() {
        let (app, _, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Shopping recommendation owner").await;
        let other_cookie = login(&app, "Shopping recommendation other").await;
        let owner_id: String = sqlx::query_scalar(
            "SELECT id FROM users WHERE display_name = 'Shopping recommendation owner'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let first = create_recipe(&app, &owner_cookie, "private").await;
        let first_id = first["id"].as_str().unwrap();
        let canonical_id = first["ingredients"][0]["canonicalIngredientId"]
            .as_str()
            .unwrap();
        let second = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({
                            "title": "Second water recipe",
                            "description": "",
                            "instructions": "",
                            "visibility": "private",
                            "ingredients": [{
                                "name": "Water",
                                "quantityNote": "",
                                "canonicalIngredientId": canonical_id
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let second = response_json(second).await;
        for recipe_id in [first_id, second["id"].as_str().unwrap()] {
            let added = app
                .clone()
                .oneshot(
                    Request::post(format!("/api/recipes/{recipe_id}/shopping-list-items"))
                        .header("content-type", "application/json")
                        .header("cookie", &owner_cookie)
                        .body(Body::from(
                            json!({
                                "name": "Water",
                                "canonicalIngredientId": canonical_id
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!([StatusCode::OK, StatusCode::CREATED].contains(&added.status()));
        }
        let shopping = app
            .clone()
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let shopping = response_json(shopping).await;
        assert_eq!(shopping.as_array().unwrap().len(), 1);
        assert_eq!(shopping[0]["sources"].as_array().unwrap().len(), 2);
        let canonical_shopping_id = shopping[0]["id"].as_str().unwrap().to_string();
        let canonical_shopping_version = shopping[0]["version"].as_i64().unwrap();

        sqlx::query(
            "INSERT INTO recipes
             (id, owner_id, title, description, instructions, visibility)
             VALUES ('unresolved-recipe', ?, 'Unresolved herb', '', '', 'private')",
        )
        .bind(&owner_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO recipe_ingredients
             (id, recipe_id, name, normalized_name, quantity_note, sort_order)
             VALUES ('unresolved-ingredient', 'unresolved-recipe', 'Mystery leaf',
                     'mystery leaf', '', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO shopping_items
             (id, owner_id, name, normalized_name, canonical_ingredient_id,
              completed_at, created_at, updated_at)
             VALUES ('unresolved-shopping', ?, 'Mystery leaf', 'mystery leaf',
                     NULL, NULL, 0, 0)",
        )
        .bind(&owner_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO shopping_item_sources
             (id, item_id, source_type, recipe_id, label, created_at)
             VALUES ('unresolved-source', 'unresolved-shopping', 'manual', NULL, '', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let recommendations = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let recommendations = response_json(recommendations).await;
        let water = recommendations
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == first["id"])
            .unwrap();
        assert_eq!(water["inShoppingList"], json!(["water"]));
        let unresolved = recommendations
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == "unresolved-recipe")
            .unwrap();
        assert_eq!(unresolved["inShoppingList"], json!(["Mystery leaf"]));

        let purchase = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/shopping-list/{canonical_shopping_id}/purchase"
                ))
                .header("content-type", "application/json")
                .header("cookie", &owner_cookie)
                .header(
                    "idempotency-key",
                    format!(
                        "{}.recommendation-membership-purchase",
                        Utc::now().timestamp()
                    ),
                )
                .body(Body::from(
                    json!({
                        "disposition": "remove",
                        "shoppingExpectedVersion": canonical_shopping_version,
                        "pantryItemId": null,
                        "createUsingCurrentName": true,
                        "pantryExpectedVersion": null,
                        "expiresOn": null,
                        "quantityNote": "",
                        "autoAddToShoppingListWhenOut": false
                    })
                    .to_string(),
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        let purchase_status = purchase.status();
        let purchase = response_json(purchase).await;
        assert_eq!(purchase_status, StatusCode::OK, "{purchase}");
        let after_purchase = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let after_purchase = response_json(after_purchase).await;
        let water_after_purchase = after_purchase
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == first["id"])
            .unwrap();
        assert_eq!(water_after_purchase["status"], "READY");
        assert_eq!(water_after_purchase["inShoppingList"], json!([]));

        let undo = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/shopping-list/purchases/{}/undo",
                    purchase["purchaseActionId"].as_str().unwrap()
                ))
                .header("cookie", &owner_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(undo.status(), StatusCode::OK);
        let after_undo = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let after_undo = response_json(after_undo).await;
        let water_after_undo = after_undo
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == first["id"])
            .unwrap();
        assert_eq!(water_after_undo["status"], "SHOP");
        assert_eq!(water_after_undo["inShoppingList"], json!(["water"]));

        let other_recipe = create_recipe(&app, &other_cookie, "private").await;
        let other_recommendations = app
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", other_cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let other_recommendations = response_json(other_recommendations).await;
        let other_result = other_recommendations
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == other_recipe["id"])
            .unwrap();
        assert_eq!(other_result["inShoppingList"], json!([]));
    }

    #[tokio::test]
    async fn recommendation_scope_uses_owner_exact_choices_and_rejects_legacy_conflicts() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Scope owner").await;
        let other_cookie = login(&app, "Scope other").await;
        let created = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({
                            "title": "Public choice",
                            "description": "",
                            "instructions": "Cook.",
                            "visibility": "public",
                            "ingredientModel": "groups-v1",
                            "ingredientGroups": [{
                                "id": "fat",
                                "optional": false,
                                "note": "",
                                "sortOrder": 0,
                                "alternatives": [
                                    {"id": "butter", "name": "butter", "quantityNote": "", "sortOrder": 0},
                                    {"id": "oil", "name": "oil", "quantityNote": "", "sortOrder": 1}
                                ]
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let created = response_json(created).await;
        let recipe_id = created["id"].as_str().unwrap();
        let added = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/shopping-list-items"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(json!({"groupId": "fat"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(added.status(), StatusCode::CREATED);

        let owner = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({"ingredientScope": "PANTRY_OR_LIST"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let owner = response_json(owner).await;
        let owner_result = owner
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == recipe_id)
            .unwrap();
        assert_eq!(owner_result["status"], "SHOP");
        assert_eq!(owner_result["rankingIntent"]["scopeEligible"], true);
        assert_eq!(
            owner_result["ingredientGroupReasons"][0]["action"],
            "SHOPPING_PLANNED"
        );
        assert_eq!(
            owner_result["appliedControls"]["ingredientScope"],
            "PANTRY_OR_LIST"
        );

        let other = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", other_cookie)
                    .body(Body::from(
                        json!({"ingredientScope": "PANTRY_OR_LIST"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let other = response_json(other).await;
        assert!(other
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["recipe"]["id"] != recipe_id));

        let conflict = app
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", owner_cookie)
                    .body(Body::from(
                        json!({
                            "ingredientScope": "PANTRY_ONLY",
                            "pantryOnly": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(conflict.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn public_author_pagination_is_stable_and_has_no_duplicates() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Author").await;
        let mut handle = String::new();
        for _ in 0..21 {
            let recipe = create_recipe(&app, &owner_cookie, "public").await;
            handle = recipe["authorHandle"].as_str().unwrap().to_string();
        }
        let first = response_json(
            app.clone()
                .oneshot(
                    Request::get(format!("/api/authors/{handle}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(first["recipes"].as_array().unwrap().len(), 20);
        let cursor = first["nextCursor"].as_str().unwrap();
        let second = response_json(
            app.oneshot(
                Request::get(format!("/api/authors/{handle}?cursor={cursor}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(second["recipes"].as_array().unwrap().len(), 1);
        assert_eq!(second["nextCursor"], Value::Null);
        let first_ids = first["recipes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|recipe| recipe["id"].as_str().unwrap())
            .collect::<std::collections::HashSet<_>>();
        assert!(!first_ids.contains(second["recipes"][0]["id"].as_str().unwrap()));
    }

    #[tokio::test]
    async fn nickname_validation_and_anonymous_onboarding_are_explicit() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Private suggestion").await;
        let anonymous = save_profile(&app, &cookie, None).await;
        assert_eq!(anonymous["nickname"], Value::Null);
        assert_eq!(anonymous["onboardingCompleted"], true);
        assert_eq!(anonymous["displayName"], "Private suggestion");

        for nickname in ["two  spaces", "line\nbreak"] {
            let response = app
                .clone()
                .oneshot(
                    Request::put("/api/settings/profile")
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .body(Body::from(json!({"nickname": nickname}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn locale_is_persisted_per_account_and_validated() {
        let app = build_app(Config::test()).await.unwrap();
        let first_cookie = login(&app, "First locale user").await;
        let second_cookie = login(&app, "Second locale user").await;

        let first = save_locale(&app, &first_cookie, "zh-CN").await;
        assert_eq!(first["locale"], "zh-CN");

        let second = app
            .clone()
            .oneshot(
                Request::get("/api/me")
                    .header("cookie", &second_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(second).await["locale"], Value::Null);

        let first = app
            .clone()
            .oneshot(
                Request::get("/api/me")
                    .header("cookie", &first_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(first).await["locale"], "zh-CN");

        let invalid = app
            .oneshot(
                Request::put("/api/settings/locale")
                    .header("content-type", "application/json")
                    .header("cookie", first_cookie)
                    .body(Body::from(json!({"locale": "fr"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn email_otp_request_is_non_enumerating_and_verification_is_one_time() {
        let (app, auth, _) = app_with_runtime().await;
        let mut shapes = Vec::new();
        let mut known_challenge = None;
        for email in ["unknown@example.test", "other@example.test"] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/api/auth/email/request")
                        .header("content-type", "application/json")
                        .header("x-forwarded-for", "203.0.113.20")
                        .body(Body::from(json!({"email": email}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = response_json(response).await;
            shapes.push(
                body.as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            if email == "unknown@example.test" {
                known_challenge = body["challengeId"].as_str().map(str::to_string);
            }
        }
        assert_eq!(shapes[0], shapes[1]);

        let challenge_id = known_challenge.unwrap();
        let code = auth
            .passwordless
            .captured_code("unknown@example.test")
            .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.20")
                    .body(Body::from(
                        json!({"challengeId": challenge_id, "code": code}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("set-cookie").is_some());

        let replay = app
            .oneshot(
                Request::post("/api/auth/email/verify")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.20")
                    .body(Body::from(
                        json!({"challengeId": challenge_id, "code": code}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_config_exposes_camel_case_passwordless_capabilities() {
        let response = build_app(Config::test())
            .await
            .unwrap()
            .oneshot(
                Request::get("/api/auth/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            json!({
                "mode": "development",
                "google": false,
                "emailOtp": true,
                "passkey": true
            })
        );
    }

    #[tokio::test]
    async fn explicit_email_link_rejects_an_identity_owned_by_another_account() {
        let (app, auth, _) = app_with_runtime().await;
        let alice_cookie = login(&app, "Alice").await;
        let bob_cookie = login(&app, "Bob").await;

        for cookie in [&alice_cookie, &bob_cookie] {
            let request = app
                .clone()
                .oneshot(
                    Request::post("/api/settings/security/email/request")
                        .header("content-type", "application/json")
                        .header("cookie", cookie)
                        .header("x-forwarded-for", "203.0.113.30")
                        .body(Body::from(
                            json!({"email": "shared@example.test"}).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(request.status(), StatusCode::OK);
            let challenge = response_json(request).await;
            let code = auth
                .passwordless
                .captured_code("shared@example.test")
                .unwrap();
            let verify = app
                .clone()
                .oneshot(
                    Request::post("/api/settings/security/email/verify")
                        .header("content-type", "application/json")
                        .header("cookie", cookie)
                        .header("x-forwarded-for", "203.0.113.30")
                        .body(Body::from(
                            json!({"challengeId": challenge["challengeId"], "code": code})
                                .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            if cookie == &alice_cookie {
                assert_eq!(verify.status(), StatusCode::OK);
            } else {
                assert_eq!(verify.status(), StatusCode::CONFLICT);
            }
        }
    }

    #[tokio::test]
    async fn google_first_email_link_rejects_mismatch_before_sending_and_accepts_exact_email() {
        let (app, auth, pool) = app_with_runtime().await;
        let cookie = login(&app, "Google Owner").await;
        let user_id: String =
            sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'Google Owner'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO auth_identities
             (issuer, subject, user_id, created_at, verified_email)
             VALUES (?, 'google-owner', ?, ?, 'owner@example.test')",
        )
        .bind(GOOGLE_IDENTITY_ISSUER)
        .bind(&user_id)
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();

        let mismatch = app
            .clone()
            .oneshot(
                Request::post("/api/settings/security/email/request")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("x-forwarded-for", "203.0.113.31")
                    .body(Body::from(
                        json!({"email": "other@example.test"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mismatch.status(), StatusCode::BAD_REQUEST);
        assert!(auth
            .passwordless
            .captured_code("other@example.test")
            .is_none());

        let request = app
            .clone()
            .oneshot(
                Request::post("/api/settings/security/email/request")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("x-forwarded-for", "203.0.113.31")
                    .body(Body::from(
                        json!({"email": " OWNER@EXAMPLE.TEST "}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(request.status(), StatusCode::OK);
        let challenge = response_json(request).await;
        let code = auth
            .passwordless
            .captured_code("owner@example.test")
            .unwrap();
        let verify = app
            .oneshot(
                Request::post("/api/settings/security/email/verify")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .header("x-forwarded-for", "203.0.113.31")
                    .body(Body::from(
                        json!({"challengeId": challenge["challengeId"], "code": code}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(verify.status(), StatusCode::OK);
        let security = response_json(verify).await;
        assert_eq!(security["linkedEmails"], json!(["owner@example.test"]));
        assert_eq!(security["googleEmail"], "owner@example.test");
        assert_eq!(security["googleSignInStatus"], "linked");
    }

    #[tokio::test]
    async fn passkey_registration_is_owner_scoped_and_requires_a_linked_email() {
        let (app, _, pool) = app_with_runtime().await;
        let alice_cookie = login(&app, "Alice").await;
        let blocked = app
            .clone()
            .oneshot(
                Request::post("/api/settings/security/passkeys/register/start")
                    .header("content-type", "application/json")
                    .header("cookie", &alice_cookie)
                    .body(Body::from(json!({"name": "Laptop"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::BAD_REQUEST);

        let alice_id: String =
            sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'Alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO auth_identities (issuer, subject, user_id, created_at)
             VALUES (?, 'alice@example.test', ?, ?)",
        )
        .bind(crate::passwordless::EMAIL_IDENTITY_ISSUER)
        .bind(&alice_id)
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();
        let allowed = app
            .oneshot(
                Request::post("/api/settings/security/passkeys/register/start")
                    .header("content-type", "application/json")
                    .header("cookie", alice_cookie)
                    .body(Body::from(json!({"name": "Laptop"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);
        assert!(allowed.headers().get("set-cookie").is_some());
        let body = response_json(allowed).await;
        assert!(body.get("publicKey").is_some());
    }

    #[tokio::test]
    async fn passkey_start_does_not_distinguish_unknown_from_no_credentials() {
        let (app, _, pool) = app_with_runtime().await;
        let alice_cookie = login(&app, "Alice").await;
        let alice_id: String =
            sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'Alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO auth_identities (issuer, subject, user_id, created_at)
             VALUES (?, 'alice@example.test', ?, ?)",
        )
        .bind(crate::passwordless::EMAIL_IDENTITY_ISSUER)
        .bind(&alice_id)
        .bind(Utc::now().timestamp())
        .execute(&pool)
        .await
        .unwrap();
        drop(alice_cookie);

        let mut responses = Vec::new();
        for email in ["unknown@example.test", "alice@example.test"] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/api/auth/passkey/start")
                        .header("content-type", "application/json")
                        .header("x-forwarded-for", "203.0.113.40")
                        .body(Body::from(json!({"email": email}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let body = response_json(response).await;
            responses.push((status, body));
        }
        assert_eq!(responses[0], responses[1]);
        assert_eq!(responses[0].0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn import_token_secret_is_one_time_and_bearer_scope_is_import_only() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let created = create_import_token(&app, &cookie).await;
        let secret = created["secret"].as_str().unwrap();
        assert!(secret.starts_with("cook_imp_"));

        let list = app
            .clone()
            .oneshot(
                Request::get("/api/settings/import-tokens")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_text = String::from_utf8(
            to_bytes(list.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(!list_text.contains(secret));
        assert!(list_text.contains("recipes:import"));

        let me = app
            .oneshot(
                Request::get("/api/me")
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn ai_capabilities_accepts_any_valid_token_without_exposing_account_data() {
        let (app, _, pool) = app_with_runtime().await;
        let cookie = login(&app, "Capability owner").await;
        let created = create_import_token(&app, &cookie).await;
        let secret = created["secret"].as_str().unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::get("/api/ai/capabilities")
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["prefix"], created["token"]["prefix"]);
        assert_eq!(body["scopes"], json!(["recipes:import"]));
        assert_eq!(body["preset"], "import");
        assert!(body["expiresAt"].as_str().is_some());
        assert!(body.get("id").is_none());
        assert!(body.get("ownerId").is_none());
        assert!(body.get("email").is_none());
        assert!(!body.to_string().contains(secret));

        let audit = sqlx::query(
            "SELECT action, token_prefix FROM ai_token_audit_events
             WHERE token_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(created["token"]["id"].as_str().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audit.get::<String, _>("action"), "token.capabilities");
        assert_eq!(
            audit.get::<String, _>("token_prefix"),
            created["token"]["prefix"].as_str().unwrap()
        );

        let unauthenticated = app
            .clone()
            .oneshot(
                Request::get("/api/ai/capabilities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let revoked = app
            .clone()
            .oneshot(
                Request::delete(format!(
                    "/api/settings/import-tokens/{}",
                    created["token"]["id"].as_str().unwrap()
                ))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
        let rejected = app
            .clone()
            .oneshot(
                Request::get("/api/ai/capabilities")
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let expired = create_manage_token(&app, &cookie).await;
        sqlx::query("UPDATE import_tokens SET expires_at = ? WHERE id = ?")
            .bind(Utc::now().timestamp() - 1)
            .bind(expired["token"]["id"].as_str().unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let expired_response = app
            .oneshot(
                Request::get("/api/ai/capabilities")
                    .header(
                        "authorization",
                        format!("Bearer {}", expired["secret"].as_str().unwrap()),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(expired_response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn scoped_ai_recipe_access_is_idempotent_owner_scoped_and_versioned() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let other_cookie = login(&app, "Other").await;
        let manage = create_manage_token(&app, &owner_cookie).await;
        let manage_secret = manage["secret"].as_str().unwrap();
        assert_eq!(
            manage["token"]["scopes"],
            json!(["recipes:read", "recipes:write", "recipes:images:write"])
        );
        let import = create_import_token(&app, &owner_cookie).await;
        let import_secret = import["secret"].as_str().unwrap();
        let denied = app
            .clone()
            .oneshot(
                Request::get("/api/ai/recipes")
                    .header("authorization", format!("Bearer {import_secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let body = json!({
            "title": "AI private soup",
            "description": "Private",
            "instructions": "Stir.",
            "sourceUrl": "https://example.test/recipes/soup",
            "sourceTitle": "Source soup",
            "sourceAuthor": "Example author",
            "sourceLicense": "CC-BY-4.0",
            "sourceAttribution": "Example author — Source soup",
            "ingredients": [{"name": "water", "quantityNote": "1 cup"}]
        });
        let create_once = |key: &'static str| {
            app.clone().oneshot(
                Request::post("/api/ai/recipes")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {manage_secret}"))
                    .header("idempotency-key", key)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
        };
        let first = create_once("test-create-0001").await.unwrap();
        assert_eq!(first.status(), StatusCode::CREATED);
        let first = response_json(first).await;
        assert_eq!(first["visibility"], "private");
        assert_eq!(first["version"], 1);
        assert!(first["difficulty"].is_null());
        assert!(first["estimatedCalories"].is_null());
        assert_eq!(first["source"]["license"], "CC-BY-4.0");
        assert_eq!(
            first["source"]["attribution"],
            "Example author — Source soup"
        );
        let recipe_id = first["id"].as_str().unwrap();
        let replay = create_once("test-create-0001").await.unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(response_json(replay).await["id"], recipe_id);

        let other_manage = create_manage_token(&app, &other_cookie).await;
        let other_secret = other_manage["secret"].as_str().unwrap();
        let cross_account = app
            .clone()
            .oneshot(
                Request::get(format!("/api/ai/recipes/{recipe_id}"))
                    .header("authorization", format!("Bearer {other_secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cross_account.status(), StatusCode::NOT_FOUND);
        let delete_not_supported = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/ai/recipes/{recipe_id}"))
                    .header("authorization", format!("Bearer {manage_secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            delete_not_supported.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );

        let update_body = json!({
            "title": "Updated private soup",
            "description": "Private",
            "instructions": "Stir twice.",
            "sourceUrl": "https://example.test/recipes/soup-v2",
            "sourceTitle": "Source soup, revised",
            "sourceAuthor": "Example author",
            "sourceLicense": "CC-BY-4.0",
            "sourceAttribution": "Example author — Source soup, revised",
            "ingredients": [{"name": "water", "quantityNote": "2 cups"}],
            "expectedVersion": 1
        });
        let update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/ai/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {manage_secret}"))
                    .body(Body::from(update_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::OK);
        let update = response_json(update).await;
        assert_eq!(update["status"], "updated");
        assert_eq!(update["recipe"]["version"], 2);
        assert!(update["recipe"]["difficulty"].is_null());
        assert!(update["recipe"]["estimatedCalories"].is_null());
        assert_eq!(
            update["recipe"]["source"]["url"],
            "https://example.test/recipes/soup-v2"
        );

        let stale = app
            .clone()
            .oneshot(
                Request::put(format!("/api/ai/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {manage_secret}"))
                    .body(Body::from(update_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        let owner_delete = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(owner_delete.status(), StatusCode::NO_CONTENT);
        let deleted_replay = create_once("test-create-0001").await.unwrap();
        assert_eq!(deleted_replay.status(), StatusCode::CONFLICT);

        let revoke = app
            .clone()
            .oneshot(
                Request::delete(format!(
                    "/api/settings/import-tokens/{}",
                    manage["token"]["id"].as_str().unwrap()
                ))
                .header("cookie", owner_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
        let revoked = app
            .oneshot(
                Request::get("/api/ai/recipes")
                    .header("authorization", format!("Bearer {manage_secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn publish_tokens_require_confirmation_and_publish_only_owned_versioned_recipes() {
        let (app, _, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Publisher").await;
        let other_cookie = login(&app, "Other").await;
        let unconfirmed = app
            .clone()
            .oneshot(
                Request::post("/api/settings/import-tokens")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({
                            "name": "Unconfirmed publisher",
                            "expiresInDays": 30,
                            "preset": "publish",
                            "confirmPublishRisk": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unconfirmed.status(), StatusCode::BAD_REQUEST);

        let manage = create_manage_token(&app, &owner_cookie).await;
        let manage_secret = manage["secret"].as_str().unwrap();
        let public_body = json!({
            "title": "Published by AI",
            "description": "Visible immediately",
            "instructions": "Stir.",
            "visibility": "public",
            "ingredients": [{"name": "water", "quantityNote": "1 cup"}]
        });
        let manage_publish = app
            .clone()
            .oneshot(
                Request::post("/api/ai/recipes")
                    .header("content-type", "application/json")
                    .header("authorization", format!("{} {}", "Bearer", manage_secret))
                    .header("idempotency-key", "manage-cannot-publish")
                    .body(Body::from(public_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(manage_publish.status(), StatusCode::FORBIDDEN);
        sqlx::query(
            "UPDATE import_tokens SET scopes = 'recipes:write recipes:publish' WHERE id = ?",
        )
        .bind(manage["token"]["id"].as_str().unwrap())
        .execute(&pool)
        .await
        .unwrap();
        let incomplete_publish_scope = app
            .clone()
            .oneshot(
                Request::post("/api/ai/recipes")
                    .header("content-type", "application/json")
                    .header("authorization", format!("{} {}", "Bearer", manage_secret))
                    .header("idempotency-key", "publish-without-read")
                    .body(Body::from(public_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(incomplete_publish_scope.status(), StatusCode::FORBIDDEN);

        let publisher = create_publish_token(&app, &owner_cookie).await;
        let secret = publisher["secret"].as_str().unwrap();
        assert_eq!(
            publisher["token"]["scopes"],
            json!([
                "recipes:read",
                "recipes:write",
                "recipes:images:write",
                "recipes:publish"
            ])
        );
        let create_public = || {
            app.clone().oneshot(
                Request::post("/api/ai/recipes")
                    .header("content-type", "application/json")
                    .header("authorization", format!("{} {}", "Bearer", secret))
                    .header("idempotency-key", "publish-create-0001")
                    .body(Body::from(public_body.to_string()))
                    .unwrap(),
            )
        };
        let created = create_public().await.unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = response_json(created).await;
        assert_eq!(created["visibility"], "public");
        let public_id = created["id"].as_str().unwrap();
        let replay = create_public().await.unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(response_json(replay).await["id"], public_id);

        let update_body = json!({
            "title": "Published by AI, updated",
            "description": "Still visible immediately",
            "instructions": "Stir twice.",
            "visibility": "public",
            "ingredients": [{"name": "water", "quantityNote": "2 cups"}],
            "expectedVersion": 1
        });
        let update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/ai/recipes/{public_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("{} {}", "Bearer", secret))
                    .body(Body::from(update_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::OK);
        let update = response_json(update).await;
        assert_eq!(update["status"], "updated");
        assert_eq!(update["recipe"]["version"], 2);
        assert_eq!(update["recipe"]["visibility"], "public");
        let stale = app
            .clone()
            .oneshot(
                Request::put(format!("/api/ai/recipes/{public_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("{} {}", "Bearer", secret))
                    .body(Body::from(update_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::CONFLICT);

        let private_body = json!({
            "title": "Private before publish",
            "description": "",
            "instructions": "Mix.",
            "ingredients": [{"name": "flour", "quantityNote": "1 cup"}]
        });
        let private = app
            .clone()
            .oneshot(
                Request::post("/api/ai/recipes")
                    .header("content-type", "application/json")
                    .header("authorization", format!("{} {}", "Bearer", secret))
                    .header("idempotency-key", "publish-create-private-0001")
                    .body(Body::from(private_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let private = response_json(private).await;
        let promoted_id = private["id"].as_str().unwrap();
        assert_eq!(private["visibility"], "private");
        let promote = app
            .clone()
            .oneshot(
                Request::put(format!("/api/ai/recipes/{promoted_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("{} {}", "Bearer", secret))
                    .body(Body::from(
                        json!({
                            "title": "Now public",
                            "description": "",
                            "instructions": "Mix.",
                            "visibility": "public",
                            "ingredients": [{"name": "flour", "quantityNote": "1 cup"}],
                            "expectedVersion": 1
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(promote.status(), StatusCode::OK);
        let promote = response_json(promote).await;
        assert_eq!(promote["recipe"]["visibility"], "public");
        assert_eq!(promote["recipe"]["version"], 2);

        let (content_type, body) = multipart_image(&test_png());
        let image = app
            .clone()
            .oneshot(
                Request::post(format!("/api/ai/recipes/{promoted_id}/cover"))
                    .header("content-type", content_type)
                    .header("authorization", format!("{} {}", "Bearer", secret))
                    .header("if-match", "\"recipe-v2\"")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(image.status(), StatusCode::OK);
        let image = response_json(image).await;
        assert_eq!(image["status"], "updated");
        assert_eq!(image["recipe"]["visibility"], "public");
        assert_eq!(image["recipe"]["version"], 3);

        let other = create_publish_token(&app, &other_cookie).await;
        let other_secret = other["secret"].as_str().unwrap();
        let cross_account = app
            .clone()
            .oneshot(
                Request::put(format!("/api/ai/recipes/{public_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("{} {}", "Bearer", other_secret))
                    .body(Body::from(
                        json!({
                            "title": "Not mine",
                            "description": "",
                            "instructions": "No.",
                            "visibility": "public",
                            "ingredients": [{"name": "water"}],
                            "expectedVersion": 2
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cross_account.status(), StatusCode::NOT_FOUND);
        let delete_not_supported = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/ai/recipes/{public_id}"))
                    .header("authorization", format!("{} {}", "Bearer", secret))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            delete_not_supported.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );

        let audit_text = sqlx::query(
            "SELECT token_prefix, action, recipe_id FROM ai_token_audit_events
             WHERE token_id = ? ORDER BY created_at",
        )
        .bind(publisher["token"]["id"].as_str().unwrap())
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            format!(
                "{} {} {}",
                row.get::<String, _>("token_prefix"),
                row.get::<String, _>("action"),
                row.get::<Option<String>, _>("recipe_id")
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
        assert!(audit_text.contains("recipe.publish.create"));
        assert!(audit_text.contains("recipe.publish.update"));
        assert!(audit_text.contains("recipe.publish"));
        assert!(audit_text.contains("recipe.image.publish.replace"));
        assert!(!audit_text.contains(secret));

        let revoke = app
            .clone()
            .oneshot(
                Request::delete(format!(
                    "/api/settings/import-tokens/{}",
                    publisher["token"]["id"].as_str().unwrap()
                ))
                .header("cookie", owner_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
        let revoked = app
            .oneshot(
                Request::get("/api/ai/recipes")
                    .header("authorization", format!("{} {}", "Bearer", secret))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn concurrent_ai_create_retries_converge_on_one_recipe() {
        let (app, _, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Owner").await;
        let manage = create_manage_token(&app, &owner_cookie).await;
        let secret = manage["secret"].as_str().unwrap();
        let body = json!({
            "title": "Concurrent soup",
            "description": "",
            "instructions": "Stir.",
            "ingredients": [{"name": "water", "quantityNote": "1 cup"}]
        })
        .to_string();
        let request = || {
            app.clone().oneshot(
                Request::post("/api/ai/recipes")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {secret}"))
                    .header("idempotency-key", "concurrent-create-0001")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
        };
        let (first, second) = tokio::join!(request(), request());
        let first = first.unwrap();
        let second = second.unwrap();
        assert!(
            [StatusCode::CREATED, StatusCode::OK].contains(&first.status()),
            "{}",
            first.status()
        );
        assert!(
            [StatusCode::CREATED, StatusCode::OK].contains(&second.status()),
            "{}",
            second.status()
        );
        let first = response_json(first).await;
        let second = response_json(second).await;
        assert_eq!(first["id"], second["id"]);
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM recipes WHERE title = 'Concurrent soup'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn public_ai_updates_require_owner_revision_acceptance() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let created = create_recipe(&app, &owner_cookie, "public").await;
        let recipe_id = created["id"].as_str().unwrap();
        let manage = create_manage_token(&app, &owner_cookie).await;
        let secret = manage["secret"].as_str().unwrap();
        let proposed = json!({
            "title": "Reviewed public soup",
            "description": "Proposed",
            "instructions": "Stir carefully.",
            "difficulty": 4,
            "estimatedCalories": 1200,
            "sourceUrl": "https://example.test/public-soup",
            "sourceTitle": "Public soup source",
            "sourceAuthor": "Source author",
            "sourceLicense": "CC-BY-4.0",
            "sourceAttribution": "Source author — Public soup source",
            "ingredients": [{"name": "water", "quantityNote": "3 cups"}],
            "expectedVersion": created["version"]
        });
        let response = app
            .clone()
            .oneshot(
                Request::put(format!("/api/ai/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::from(proposed.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let pending = response_json(response).await;
        assert_eq!(pending["status"], "pendingReview");
        assert_eq!(pending["revision"]["difficulty"], 4);
        assert_eq!(pending["revision"]["estimatedCalories"], 1200);
        assert_eq!(pending["revision"]["sourceLicense"], "CC-BY-4.0");
        let revision_id = pending["revision"]["id"].as_str().unwrap();

        let live = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(live).await["title"], "Test soup");

        let accept = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipe-revisions/{revision_id}/accept"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accept.status(), StatusCode::OK);
        let accepted = response_json(accept).await;
        assert_eq!(accepted["title"], "Reviewed public soup");
        assert_eq!(accepted["difficulty"], 4);
        assert_eq!(accepted["estimatedCalories"], 1200);
        assert_eq!(
            accepted["source"]["url"],
            "https://example.test/public-soup"
        );
        assert_eq!(accepted["source"]["license"], "CC-BY-4.0");
        assert_eq!(accepted["version"], 2);
        assert_eq!(accepted["visibility"], "public");

        let duplicate = app
            .oneshot(
                Request::post(format!("/api/recipe-revisions/{revision_id}/accept"))
                    .header("cookie", owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(duplicate.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn recipe_images_are_normalized_private_and_revision_controlled() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let created = create_recipe(&app, &owner_cookie, "private").await;
        let recipe_id = created["id"].as_str().unwrap();
        let first_token = create_manage_token(&app, &owner_cookie).await;
        let first_secret = first_token["secret"].as_str().unwrap();
        let (oversized_type, oversized_body) =
            multipart_image(&vec![b'x'; crate::images::MAX_UPLOAD_BYTES + 128 * 1024]);
        let oversized = app
            .clone()
            .oneshot(
                Request::post(format!("/api/ai/recipes/{recipe_id}/cover"))
                    .header("content-type", oversized_type)
                    .header("authorization", format!("Bearer {first_secret}"))
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(oversized_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let (content_type, body) = multipart_image(&test_png());
        let upload = app
            .clone()
            .oneshot(
                Request::post(format!("/api/ai/recipes/{recipe_id}/cover"))
                    .header("content-type", content_type)
                    .header("authorization", format!("Bearer {first_secret}"))
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::OK);
        let uploaded = response_json(upload).await;
        assert_eq!(uploaded["status"], "updated");
        assert_eq!(uploaded["recipe"]["version"], 2);
        assert_eq!(uploaded["recipe"]["coverImage"]["altText"], "Blue soup");
        let image_url = uploaded["recipe"]["coverImage"]["url"]
            .as_str()
            .unwrap()
            .to_string();

        let anonymous = app
            .clone()
            .oneshot(Request::get(&image_url).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(anonymous.status(), StatusCode::NOT_FOUND);
        let owner_token_read = app
            .clone()
            .oneshot(
                Request::get(&image_url)
                    .header("authorization", format!("Bearer {first_secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(owner_token_read.status(), StatusCode::OK);
        assert_eq!(owner_token_read.headers()["content-type"], "image/jpeg");
        assert_eq!(
            owner_token_read.headers()["cache-control"],
            "private, no-store"
        );
        assert_eq!(
            owner_token_read.headers()["x-content-type-options"],
            "nosniff"
        );
        let etag = owner_token_read.headers()["etag"]
            .to_str()
            .unwrap()
            .to_string();
        let not_modified = app
            .clone()
            .oneshot(
                Request::get(&image_url)
                    .header("authorization", format!("Bearer {first_secret}"))
                    .header("if-none-match", etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
        let metadata_update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .header("if-match", "\"recipe-v2\"")
                    .header("idempotency-key", cover_key("metadata-private-image"))
                    .body(Body::from(
                        json!({
                            "altText": "Updated blue soup",
                            "sourceUrl": "",
                            "sourceLicense": "",
                            "sourceAttribution": ""
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(metadata_update.status(), StatusCode::OK);
        assert_eq!(response_json(metadata_update).await["version"], 3);
        let metadata_recipe = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let metadata_recipe = response_json(metadata_recipe).await;
        assert_eq!(metadata_recipe["version"], 3);
        assert_eq!(
            metadata_recipe["coverImage"]["altText"],
            "Updated blue soup"
        );

        let revoke = app
            .clone()
            .oneshot(
                Request::delete(format!(
                    "/api/settings/import-tokens/{}",
                    first_token["token"]["id"].as_str().unwrap()
                ))
                .header("cookie", &owner_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
        let revoked_read = app
            .clone()
            .oneshot(
                Request::get(&image_url)
                    .header("authorization", format!("Bearer {first_secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked_read.status(), StatusCode::UNAUTHORIZED);

        let publish = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .header("if-match", "\"recipe-v3\"")
                    .body(Body::from(
                        json!({
                            "title": "Test soup",
                            "description": "",
                            "instructions": "",
                            "visibility": "public",
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(publish.status(), StatusCode::OK);
        assert_eq!(response_json(publish).await["version"], 4);
        let public_read = app
            .clone()
            .oneshot(
                Request::get(&image_url)
                    .header("range", "bytes=0-9")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(public_read.status(), StatusCode::OK);
        assert_eq!(
            public_read.headers()["cache-control"],
            "public, max-age=0, must-revalidate"
        );
        let public_etag = public_read.headers()["etag"].clone();
        let make_private = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .header("if-match", "\"recipe-v4\"")
                    .body(Body::from(
                        json!({
                            "title": "Test soup",
                            "description": "",
                            "instructions": "",
                            "visibility": "private",
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(make_private.status(), StatusCode::OK);
        let private_recipe = response_json(make_private).await;
        assert_eq!(private_recipe["version"], 5);
        let private_url = private_recipe["coverImage"]["url"].as_str().unwrap();
        assert_ne!(private_url, image_url);
        let retired_public_url = app
            .clone()
            .oneshot(
                Request::get(&image_url)
                    .header("if-none-match", public_etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retired_public_url.status(), StatusCode::NOT_FOUND);
        let anonymous_private = app
            .clone()
            .oneshot(Request::get(private_url).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(anonymous_private.status(), StatusCode::NOT_FOUND);
        let republish = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .header("if-match", "\"recipe-v5\"")
                    .body(Body::from(
                        json!({
                            "title": "Test soup",
                            "description": "",
                            "instructions": "",
                            "visibility": "public",
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(republish.status(), StatusCode::OK);
        assert_eq!(response_json(republish).await["version"], 6);

        let second_token = create_manage_token(&app, &owner_cookie).await;
        let second_secret = second_token["secret"].as_str().unwrap();
        let (replacement_type, replacement_body) = multipart_image(&test_png());
        let proposed_replace = app
            .clone()
            .oneshot(
                Request::post(format!("/api/ai/recipes/{recipe_id}/cover"))
                    .header("content-type", replacement_type)
                    .header("authorization", format!("Bearer {second_secret}"))
                    .header("if-match", "\"recipe-v6\"")
                    .body(Body::from(replacement_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(proposed_replace.status(), StatusCode::OK);
        let proposed_replace = response_json(proposed_replace).await;
        assert_eq!(proposed_replace["status"], "pendingReview");
        assert_eq!(proposed_replace["revision"]["imageAction"], "replace");
        let replace_revision_id = proposed_replace["revision"]["id"].as_str().unwrap();
        let preview = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipe-revisions/{replace_revision_id}/cover"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK);
        assert_eq!(preview.headers()["cache-control"], "private, no-store");
        let accept_replace = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/recipe-revisions/{replace_revision_id}/accept"
                ))
                .header("cookie", &owner_cookie)
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accept_replace.status(), StatusCode::OK);
        let accepted_replace = response_json(accept_replace).await;
        assert_eq!(accepted_replace["version"], 7);
        assert_ne!(accepted_replace["coverImage"]["url"], private_url);
        let retired_url = app
            .clone()
            .oneshot(Request::get(private_url).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(retired_url.status(), StatusCode::NOT_FOUND);

        let proposed_delete = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/ai/recipes/{recipe_id}/cover"))
                    .header("authorization", format!("Bearer {second_secret}"))
                    .header("if-match", "\"recipe-v7\"")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(proposed_delete.status(), StatusCode::OK);
        let proposed_delete = response_json(proposed_delete).await;
        assert_eq!(proposed_delete["status"], "pendingReview");
        assert_eq!(proposed_delete["revision"]["imageAction"], "delete");
        let revision_id = proposed_delete["revision"]["id"].as_str().unwrap();
        let accepted = app
            .oneshot(
                Request::post(format!("/api/recipe-revisions/{revision_id}/accept"))
                    .header("cookie", owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let accepted_status = accepted.status();
        let accepted = response_json(accepted).await;
        assert_eq!(accepted_status, StatusCode::OK, "{accepted}");
        assert_eq!(accepted["version"], 8);
        assert!(accepted["coverImage"].is_null());
    }

    #[tokio::test]
    async fn import_drafts_are_owner_scoped_and_accept_exactly_once() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let other_cookie = login(&app, "Other").await;
        let token = create_import_token(&app, &owner_cookie).await;
        let secret = token["secret"].as_str().unwrap();
        let response =
            submit_import(&app, secret, import_payload("https://example.test/soup")).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let draft = response_json(response).await;
        let draft_id = draft["id"].as_str().unwrap();
        assert_eq!(draft["difficulty"], 2);
        assert_eq!(draft["estimatedCalories"], 450);
        assert_eq!(draft["sourceLicense"], "CC-BY-4.0");

        let mut optional_metadata = import_payload("https://example.test/optional-metadata-soup");
        optional_metadata
            .as_object_mut()
            .unwrap()
            .remove("difficulty");
        optional_metadata
            .as_object_mut()
            .unwrap()
            .insert("estimatedCalories".into(), Value::Null);
        let optional = submit_import(&app, secret, optional_metadata).await;
        assert_eq!(optional.status(), StatusCode::CREATED);
        let optional = response_json(optional).await;
        assert!(optional["difficulty"].is_null());
        assert!(optional["estimatedCalories"].is_null());

        let other = app
            .clone()
            .oneshot(
                Request::get(format!("/api/import-drafts/{draft_id}"))
                    .header("cookie", &other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(other.status(), StatusCode::NOT_FOUND);
        let other_accept = app
            .clone()
            .oneshot(
                Request::post(format!("/api/import-drafts/{draft_id}/accept"))
                    .header("cookie", other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(other_accept.status(), StatusCode::NOT_FOUND);

        let accept = app
            .clone()
            .oneshot(
                Request::post(format!("/api/import-drafts/{draft_id}/accept"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accept.status(), StatusCode::CREATED);
        let recipe = response_json(accept).await;
        assert_eq!(recipe["visibility"], "private");
        assert_eq!(recipe["source"]["url"], "https://example.test/soup");
        assert_eq!(recipe["source"]["license"], "CC-BY-4.0");
        assert_eq!(
            recipe["source"]["attribution"],
            "Example author — Example soup"
        );
        assert_eq!(recipe["source"]["warnings"][0], "Timing was inferred.");
        assert_eq!(recipe["difficulty"], 2);
        assert_eq!(recipe["estimatedCalories"], 450);

        let duplicate = app
            .oneshot(
                Request::post(format!("/api/import-drafts/{draft_id}/accept"))
                    .header("cookie", owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn grouped_ids_are_resource_scoped_and_legacy_updates_cannot_flatten_import_drafts() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Grouped owner").await;
        let recipe_payload = json!({
            "title": "Scoped IDs",
            "description": "",
            "instructions": "Cook.",
            "visibility": "private",
            "ingredientModel": "groups-v1",
            "ingredientGroups": [{
                "id": "reusable-group",
                "optional": false,
                "note": "group note",
                "sortOrder": 0,
                "alternatives": [{
                    "id": "reusable-alternative",
                    "name": "beans",
                    "quantityNote": "2 cups",
                    "sortOrder": 0
                }]
            }]
        });
        let mut first_recipe = None;
        for title in ["First scoped recipe", "Second scoped recipe"] {
            let mut payload = recipe_payload.clone();
            payload["title"] = json!(title);
            let created = app
                .clone()
                .oneshot(
                    Request::post("/api/recipes")
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .body(Body::from(payload.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(created.status(), StatusCode::CREATED);
            let created = response_json(created).await;
            assert_eq!(created["ingredientGroups"][0]["id"], "reusable-group");
            assert_eq!(
                created["ingredientGroups"][0]["alternatives"][0]["id"],
                "reusable-alternative"
            );
            assert_eq!(created["ingredients"][0]["quantityNote"], "2 cups");
            if first_recipe.is_none() {
                first_recipe = Some((
                    created["id"].as_str().unwrap().to_string(),
                    created["version"].as_i64().unwrap(),
                ));
            }
        }
        let (first_recipe_id, first_recipe_version) = first_recipe.unwrap();
        let legacy_recipe_update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{first_recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", format!("\"recipe-v{first_recipe_version}\""))
                    .body(Body::from(
                        json!({
                            "title": "Legacy flatten attempt",
                            "description": "",
                            "instructions": "Cook.",
                            "visibility": "private",
                            "ingredients": [{"name": "beans", "quantityNote": "2 cups"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(legacy_recipe_update.status(), StatusCode::BAD_REQUEST);

        let token = create_import_token(&app, &cookie).await;
        let secret = token["secret"].as_str().unwrap();
        let grouped_import = json!({
            "schemaVersion": "2.0",
            "sourceUrl": "https://example.test/grouped",
            "title": "Grouped draft",
            "ingredientModel": "groups-v1",
            "ingredientGroups": [{
                "id": "reusable-group",
                "optional": true,
                "note": "optional spread",
                "sortOrder": 0,
                "alternatives": [
                    {"id": "reusable-alternative", "name": "butter", "quantityNote": "1 tbsp", "sortOrder": 0},
                    {"id": "oil", "name": "oil", "quantityNote": "2 tbsp", "sortOrder": 1}
                ]
            }],
            "steps": ["Cook."]
        });
        let first = submit_import(&app, secret, grouped_import.clone()).await;
        assert_eq!(first.status(), StatusCode::CREATED);
        let first = response_json(first).await;
        let draft_id = first["id"].as_str().unwrap();
        let second = submit_import(&app, secret, grouped_import).await;
        assert_eq!(second.status(), StatusCode::CREATED);
        let single = submit_import(
            &app,
            secret,
            json!({
                "schemaVersion": "2.0",
                "sourceUrl": "https://example.test/single-grouped",
                "title": "Single grouped draft",
                "ingredientModel": "groups-v1",
                "ingredientGroups": [{
                    "id": "reusable-group",
                    "optional": false,
                    "note": "draft group note",
                    "sortOrder": 0,
                    "alternatives": [{
                        "id": "reusable-alternative",
                        "name": "lentils",
                        "quantityNote": "3 cups",
                        "sortOrder": 0
                    }]
                }],
                "steps": ["Cook."]
            }),
        )
        .await;
        assert_eq!(single.status(), StatusCode::CREATED);
        let single = response_json(single).await;
        assert_eq!(single["ingredients"][0]["quantityNote"], "3 cups");
        let single_draft_id = single["id"].as_str().unwrap();
        let single_legacy_update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/import-drafts/{single_draft_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Flattened single",
                            "description": "",
                            "instructions": "Cook.",
                            "ingredients": [{"name": "lentils", "quantityNote": "3 cups"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(single_legacy_update.status(), StatusCode::BAD_REQUEST);

        let legacy_update = app
            .oneshot(
                Request::put(format!("/api/import-drafts/{draft_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!({
                            "title": "Flattened",
                            "description": "",
                            "instructions": "Cook.",
                            "ingredients": [{"name": "butter", "quantityNote": "1 tbsp"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(legacy_update.status(), StatusCode::BAD_REQUEST);
        assert!(String::from_utf8(
            to_bytes(legacy_update.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec()
        )
        .unwrap()
        .contains("ingredient_groups_required"));
    }

    #[tokio::test]
    async fn revoked_expired_and_malformed_imports_are_rejected() {
        let path = std::env::temp_dir().join(format!("cook-import-{}.db", Uuid::new_v4()));
        let mut config = Config::test();
        config.database_url = format!("sqlite://{}", path.display());
        let app = build_app(config.clone()).await.unwrap();
        let pool = crate::db::connect(&config.database_url).await.unwrap();
        let cookie = login(&app, "Owner").await;

        let malformed_token = create_import_token(&app, &cookie).await;
        let malformed_secret = malformed_token["secret"].as_str().unwrap();
        let malformed =
            submit_import(&app, malformed_secret, import_payload("file:///etc/passwd")).await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        let oversized = json!({
            "schemaVersion": "1.0",
            "sourceUrl": "https://example.test/recipe",
            "title": "Soup",
            "ingredients": (0..101).map(|i| json!({"name": format!("item {i}"), "quantityNote": ""})).collect::<Vec<_>>(),
            "steps": ["Stir."]
        });
        let oversized = submit_import(&app, malformed_secret, oversized).await;
        assert_eq!(oversized.status(), StatusCode::BAD_REQUEST);
        let mut owner_injection = import_payload("https://example.test/owner-injection");
        owner_injection["ownerId"] = json!("another-user");
        let owner_injection = submit_import(&app, malformed_secret, owner_injection).await;
        assert_eq!(owner_injection.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let revoked = create_import_token(&app, &cookie).await;
        let revoked_secret = revoked["secret"].as_str().unwrap();
        let token_id = revoked["token"]["id"].as_str().unwrap();
        let revoke = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/settings/import-tokens/{token_id}"))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
        let rejected = submit_import(
            &app,
            revoked_secret,
            import_payload("https://example.test/revoked"),
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let expired = create_import_token(&app, &cookie).await;
        let expired_secret = expired["secret"].as_str().unwrap();
        let expired_id = expired["token"]["id"].as_str().unwrap();
        sqlx::query("UPDATE import_tokens SET expires_at = ? WHERE id = ?")
            .bind(Utc::now().timestamp() - 1)
            .bind(expired_id)
            .execute(&pool)
            .await
            .unwrap();
        let rejected = submit_import(
            &app,
            expired_secret,
            import_payload("https://example.test/expired"),
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        pool.close().await;
    }

    #[tokio::test]
    async fn import_token_rate_limit_is_enforced() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let token = create_import_token(&app, &cookie).await;
        let secret = token["secret"].as_str().unwrap();
        for index in 0..20 {
            let response = submit_import(
                &app,
                secret,
                import_payload(&format!("https://example.test/{index}")),
            )
            .await;
            assert_eq!(response.status(), StatusCode::CREATED);
        }
        let limited =
            submit_import(&app, secret, import_payload("https://example.test/limited")).await;
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn another_account_cannot_update_or_delete_public_recipe() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let other_cookie = login(&app, "Other").await;
        let created = create_recipe(&app, &owner_cookie, "public").await;
        let path = format!("/api/recipes/{}", created["id"].as_str().unwrap());
        let update = app
            .clone()
            .oneshot(
                Request::put(&path)
                    .header("content-type", "application/json")
                    .header("cookie", &other_cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(
                        json!({
                            "title": "Hijacked",
                            "description": "",
                            "instructions": "",
                            "visibility": "public",
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::NOT_FOUND);
        let delete = app
            .oneshot(
                Request::delete(&path)
                    .header("cookie", other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn pantry_is_scoped_to_the_current_account() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let other_cookie = login(&app, "Other").await;
        let update = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", owner_cookie)
                    .body(Body::from(
                        json!([{"name": "egg", "status": "have", "quantityNote": ""}]).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::OK);
        let response = app
            .oneshot(
                Request::get("/api/pantry")
                    .header("cookie", other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let pantry: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pantry, json!([]));
    }

    #[tokio::test]
    async fn recipe_metadata_is_optional_validated_and_owner_editable() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let create = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Metadata soup",
                            "description": "",
                            "instructions": "Stir.",
                            "visibility": "private",
                            "difficulty": 3,
                            "estimatedCalories": 900,
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status(), StatusCode::CREATED);
        let recipe = response_json(create).await;
        assert_eq!(recipe["difficulty"], 3);
        assert_eq!(recipe["estimatedCalories"], 900);
        let recipe_id = recipe["id"].as_str().unwrap();

        let clear = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(
                        json!({
                            "title": "Metadata soup",
                            "description": "",
                            "instructions": "Stir.",
                            "visibility": "private",
                            "difficulty": null,
                            "estimatedCalories": null,
                            "ingredients": [{"name": "water", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(clear.status(), StatusCode::OK);
        let cleared = response_json(clear).await;
        assert!(cleared["difficulty"].is_null());
        assert!(cleared["estimatedCalories"].is_null());

        let omitted = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Metadata omitted",
                            "visibility": "private",
                            "ingredients": [{"name": "water"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(omitted.status(), StatusCode::CREATED);
        let omitted = response_json(omitted).await;
        assert!(omitted["difficulty"].is_null());
        assert!(omitted["estimatedCalories"].is_null());

        let invalid = app
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!({
                            "title": "Invalid metadata",
                            "visibility": "private",
                            "difficulty": 0,
                            "estimatedCalories": 100001,
                            "ingredients": [{"name": "water"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn interactive_recipe_and_image_mutations_reject_stale_versions() {
        let (app, _, pool) = app_with_runtime().await;
        let cookie = login(&app, "Owner").await;
        let created = create_recipe(&app, &cookie, "private").await;
        let recipe_id = created["id"].as_str().unwrap();
        let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM recipes WHERE id = ?")
            .bind(recipe_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let browser_update = json!({
            "title": "Browser edit",
            "description": "",
            "instructions": "Stir.",
            "visibility": "private",
            "ingredients": [{"name": "water", "quantityNote": ""}]
        });
        let first = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(browser_update.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(response_json(first).await["version"], 2);
        let stale_tab = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(browser_update.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale_tab.status(), StatusCode::CONFLICT);

        let token = create_manage_token(&app, &cookie).await;
        let ai_update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/ai/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        format!("Bearer {}", token["secret"].as_str().unwrap()),
                    )
                    .body(Body::from(
                        json!({
                            "title": "AI edit",
                            "description": "",
                            "instructions": "Stir twice.",
                            "ingredients": [{"name": "water", "quantityNote": ""}],
                            "expectedVersion": 2
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ai_update.status(), StatusCode::OK);
        assert_eq!(response_json(ai_update).await["recipe"]["version"], 3);
        let stale_browser = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v2\"")
                    .body(Body::from(browser_update.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale_browser.status(), StatusCode::CONFLICT);

        let (content_type, body) = multipart_image(&test_png());
        let upload_key = cover_key("interactive-upload");
        let upload = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", content_type)
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v3\"")
                    .header("idempotency-key", upload_key.as_str())
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::OK);
        assert_eq!(response_json(upload).await["version"], 4);
        set_cover_rate_count(&pool, &owner_id, 30).await;
        let (replay_content_type, replay_body) = multipart_image(&test_png());
        let replay = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", replay_content_type)
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v3\"")
                    .header("idempotency-key", upload_key.as_str())
                    .body(Body::from(replay_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(response_json(replay).await["version"], 4);
        let (different_content_type, different_body) =
            multipart_image_with_alt(&test_png(), "Different cover");
        let reused_for_different_upload = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", different_content_type)
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v4\"")
                    .header("idempotency-key", upload_key.as_str())
                    .body(Body::from(different_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reused_for_different_upload.status(), StatusCode::CONFLICT);
        set_cover_rate_count(&pool, &owner_id, 0).await;
        let stale_content_after_image = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v3\"")
                    .body(Body::from(browser_update.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale_content_after_image.status(), StatusCode::CONFLICT);

        let metadata = json!({
            "altText": "Updated cover",
            "sourceUrl": "",
            "sourceLicense": "",
            "sourceAttribution": ""
        });
        let metadata_key = cover_key("interactive-metadata");
        let metadata_update = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v4\"")
                    .header("idempotency-key", metadata_key.as_str())
                    .body(Body::from(metadata.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(metadata_update.status(), StatusCode::OK);
        assert_eq!(response_json(metadata_update).await["version"], 5);
        set_cover_rate_count(&pool, &owner_id, 30).await;
        let metadata_replay = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v4\"")
                    .header("idempotency-key", metadata_key.as_str())
                    .body(Body::from(metadata.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(metadata_replay.status(), StatusCode::OK);
        assert_eq!(response_json(metadata_replay).await["version"], 5);
        set_cover_rate_count(&pool, &owner_id, 0).await;
        let stale_metadata = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v4\"")
                    .header("idempotency-key", cover_key("stale-interactive-metadata"))
                    .body(Body::from(metadata.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale_metadata.status(), StatusCode::CONFLICT);
        let delete_key = cover_key("interactive-delete");
        let deleted = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}/cover"))
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v5\"")
                    .header("idempotency-key", delete_key.as_str())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::OK);
        assert_eq!(response_json(deleted).await["version"], 6);
        set_cover_rate_count(&pool, &owner_id, 30).await;
        let delete_replay = app
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}/cover"))
                    .header("cookie", cookie)
                    .header("if-match", "\"recipe-v5\"")
                    .header("idempotency-key", delete_key.as_str())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_replay.status(), StatusCode::OK);
        assert_eq!(response_json(delete_replay).await["version"], 6);
    }

    #[tokio::test]
    async fn concurrent_cover_mutations_converge_by_idempotency_key() {
        let path = std::env::temp_dir().join(format!("cook-cover-race-{}.db", Uuid::new_v4()));
        let mut config = Config::test();
        config.database_url = format!("sqlite://{}", path.display());
        let app = build_app(config.clone()).await.unwrap();
        let pool = crate::db::connect(&config.database_url).await.unwrap();
        let cookie = login(&app, "Cover race owner").await;
        let recipe = create_recipe(&app, &cookie, "private").await;
        let recipe_id = recipe["id"].as_str().unwrap();

        let upload_key = cover_key("concurrent-upload");
        let (upload_type_a, upload_body_a) = multipart_image(&test_png());
        let (upload_type_b, upload_body_b) = multipart_image(&test_png());
        let (upload_a, upload_b) = tokio::join!(
            app.clone().oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", upload_type_a)
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .header("idempotency-key", upload_key.as_str())
                    .body(Body::from(upload_body_a))
                    .unwrap()
            ),
            app.clone().oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", upload_type_b)
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .header("idempotency-key", upload_key.as_str())
                    .body(Body::from(upload_body_b))
                    .unwrap()
            )
        );
        for response in [upload_a.unwrap(), upload_b.unwrap()] {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response_json(response).await["version"], 2);
        }

        let metadata = json!({
            "altText": "Concurrent metadata",
            "sourceUrl": "",
            "sourceLicense": "",
            "sourceAttribution": ""
        });
        let metadata_key = cover_key("concurrent-metadata");
        let (metadata_a, metadata_b) = tokio::join!(
            app.clone().oneshot(
                Request::put(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v2\"")
                    .header("idempotency-key", metadata_key.as_str())
                    .body(Body::from(metadata.to_string()))
                    .unwrap()
            ),
            app.clone().oneshot(
                Request::put(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v2\"")
                    .header("idempotency-key", metadata_key.as_str())
                    .body(Body::from(metadata.to_string()))
                    .unwrap()
            )
        );
        for response in [metadata_a.unwrap(), metadata_b.unwrap()] {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response_json(response).await["version"], 3);
        }

        let delete_key = cover_key("concurrent-delete");
        let (delete_a, delete_b) = tokio::join!(
            app.clone().oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}/cover"))
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v3\"")
                    .header("idempotency-key", delete_key.as_str())
                    .body(Body::empty())
                    .unwrap()
            ),
            app.clone().oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}/cover"))
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v3\"")
                    .header("idempotency-key", delete_key.as_str())
                    .body(Body::empty())
                    .unwrap()
            )
        );
        for response in [delete_a.unwrap(), delete_b.unwrap()] {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response_json(response).await["version"], 4);
        }

        let changed_key = cover_key("concurrent-changed-upload");
        let (changed_type_a, changed_body_a) =
            multipart_image_with_alt(&test_png(), "First concurrent cover");
        let (changed_type_b, changed_body_b) =
            multipart_image_with_alt(&test_png(), "Second concurrent cover");
        let (changed_a, changed_b) = tokio::join!(
            app.clone().oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", changed_type_a)
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v4\"")
                    .header("idempotency-key", changed_key.as_str())
                    .body(Body::from(changed_body_a))
                    .unwrap()
            ),
            app.clone().oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/cover"))
                    .header("content-type", changed_type_b)
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v4\"")
                    .header("idempotency-key", changed_key.as_str())
                    .body(Body::from(changed_body_b))
                    .unwrap()
            )
        );
        let mut statuses = [changed_a.unwrap().status(), changed_b.unwrap().status()];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
        let version: i64 = sqlx::query_scalar("SELECT version FROM recipes WHERE id = ?")
            .bind(recipe_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 5);
    }

    #[tokio::test]
    async fn cover_mutation_retention_capacity_and_rate_limit_are_bounded() {
        let (app, _, pool) = app_with_runtime().await;
        let cookie = login(&app, "Cover limits owner").await;
        let recipe = create_recipe(&app, &cookie, "private").await;
        let recipe_id = recipe["id"].as_str().unwrap();
        let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM recipes WHERE id = ?")
            .bind(recipe_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let now = Utc::now().timestamp();

        let expired = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}/cover"))
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .header("idempotency-key", format!("{}.expired-key", now - 86_401))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(expired.status(), StatusCode::CONFLICT);

        let mut transaction = pool.begin().await.unwrap();
        for index in 0..300 {
            sqlx::query(
                "INSERT INTO recipe_cover_mutations
                 (owner_id, recipe_id, idempotency_key, operation, request_fingerprint,
                  result_version, created_at)
                 VALUES (?, ?, ?, 'delete', X'00', 1, ?)",
            )
            .bind(&owner_id)
            .bind(recipe_id)
            .bind(format!("expired-{index}"))
            .bind(now - 86_401)
            .execute(&mut *transaction)
            .await
            .unwrap();
        }
        transaction.commit().await.unwrap();

        let valid_delete = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}/cover"))
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .header("idempotency-key", cover_key("cleanup-delete"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(valid_delete.status(), StatusCode::OK);
        let expired_remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM recipe_cover_mutations WHERE created_at < ?")
                .bind(now - 86_400)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(expired_remaining, 44);

        let mut transaction = pool.begin().await.unwrap();
        for index in 0..127 {
            sqlx::query(
                "INSERT INTO recipe_cover_mutations
                 (owner_id, recipe_id, idempotency_key, operation, request_fingerprint,
                  result_version, created_at)
                 VALUES (?, ?, ?, 'delete', X'01', 2, ?)",
            )
            .bind(&owner_id)
            .bind(recipe_id)
            .bind(format!("active-{index}"))
            .bind(now)
            .execute(&mut *transaction)
            .await
            .unwrap();
        }
        transaction.commit().await.unwrap();
        let capped = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/recipes/{recipe_id}/cover"))
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v2\"")
                    .header("idempotency-key", cover_key("capacity-delete"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(capped.status(), StatusCode::TOO_MANY_REQUESTS);

        let rate_cookie = login(&app, "Cover rate owner").await;
        let rate_recipe = create_recipe(&app, &rate_cookie, "private").await;
        let rate_recipe_id = rate_recipe["id"].as_str().unwrap();
        let rate_owner_id: String = sqlx::query_scalar("SELECT owner_id FROM recipes WHERE id = ?")
            .bind(rate_recipe_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        set_cover_rate_count(&pool, &rate_owner_id, 30).await;
        let rate_limited = app
            .oneshot(
                Request::delete(format!("/api/recipes/{rate_recipe_id}/cover"))
                    .header("cookie", &rate_cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .header("idempotency-key", cover_key("rate-delete"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rate_limited.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn automatic_shopping_items_follow_out_pantry_state_and_purchase_atomically() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let other_cookie = login(&app, "Other").await;
        let pantry = app
            .clone()
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!([
                            {
                                "name": "Milk",
                                "status": "out",
                                "quantityNote": "",
                                "autoAddToShoppingListWhenOut": true
                            },
                            {
                                "name": "Egg",
                                "status": "unknown",
                                "quantityNote": "",
                                "autoAddToShoppingListWhenOut": true
                            },
                            {
                                "name": "Rice",
                                "status": "have",
                                "quantityNote": "",
                                "expiresOn": "2020-01-01",
                                "autoAddToShoppingListWhenOut": true
                            }
                        ])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pantry.status(), StatusCode::OK);
        let pantry = response_json(pantry).await;
        let milk = pantry
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Milk")
            .unwrap();
        let milk_id = milk["id"].as_str().unwrap().to_string();
        let milk_canonical_id = milk["canonicalIngredientId"].as_str().unwrap();
        let manual = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({
                            "name": "milk",
                            "canonicalIngredientId": milk_canonical_id
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(manual.status(), StatusCode::CREATED);

        let list = app
            .clone()
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list = response_json(list).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["name"], "milk");
        assert_eq!(list[0]["automatic"], true);
        assert_eq!(list[0]["pantryItemId"], milk_id);
        let shopping_id = list[0]["id"].as_str().unwrap().to_string();
        let shopping_version = list[0]["version"].as_i64().unwrap();
        let pantry_version = milk["version"].as_i64().unwrap();
        assert_eq!(list[0]["sources"].as_array().unwrap().len(), 2);
        assert!(list[0]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["kind"] == "automatic"));
        assert!(list[0]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["kind"] == "manual"));

        let other_list = app
            .clone()
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", &other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(other_list).await, json!([]));
        let cross_account = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/{shopping_id}/purchase"))
                    .header("cookie", &other_cookie)
                    .header("content-type", "application/json")
                    .header(
                        "idempotency-key",
                        format!("{}.cross-account", Utc::now().timestamp()),
                    )
                    .body(Body::from(
                        json!({
                            "disposition": "remove",
                            "shoppingExpectedVersion": shopping_version,
                            "pantryItemId": milk_id,
                            "pantryExpectedVersion": pantry_version,
                            "expiresOn": null,
                            "quantityNote": "",
                            "autoAddToShoppingListWhenOut": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cross_account.status(), StatusCode::NOT_FOUND);

        let purchase_key = format!("{}.purchase", Utc::now().timestamp());
        let purchase_body = json!({
            "disposition": "remove",
            "shoppingExpectedVersion": shopping_version,
            "pantryItemId": milk_id,
            "pantryExpectedVersion": pantry_version,
            "expiresOn": null,
            "quantityNote": "",
            "autoAddToShoppingListWhenOut": true
        })
        .to_string();
        let purchased = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/{shopping_id}/purchase"))
                    .header("cookie", &owner_cookie)
                    .header("content-type", "application/json")
                    .header("idempotency-key", &purchase_key)
                    .body(Body::from(purchase_body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(purchased.status(), StatusCode::OK);
        let purchased = response_json(purchased).await;
        assert!(purchased["shoppingItems"].as_array().unwrap().is_empty());
        assert!(purchased["purchaseActionId"].is_string());
        let action_id = purchased["purchaseActionId"].as_str().unwrap();
        let replay = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/{shopping_id}/purchase"))
                    .header("cookie", &owner_cookie)
                    .header("content-type", "application/json")
                    .header("idempotency-key", &purchase_key)
                    .body(Body::from(purchase_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(response_json(replay).await["purchaseActionId"], action_id);
        let pantry = app
            .clone()
            .oneshot(
                Request::get("/api/pantry")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let pantry = response_json(pantry).await;
        let milk = pantry
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == milk_id)
            .unwrap();
        assert_eq!(milk["status"], "have");
        assert!(milk["confirmedAt"].is_string());
        assert_eq!(milk["autoAddToShoppingListWhenOut"], true);
        let undo = app
            .oneshot(
                Request::post(format!("/api/shopping-list/purchases/{action_id}/undo"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(undo.status(), StatusCode::OK);
        let restored = response_json(undo).await;
        assert_eq!(restored.as_array().unwrap().len(), 1);
        assert_eq!(restored[0]["sources"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn shopping_purchase_handles_automatic_keep_cross_item_keys_and_undo_identity_conflicts()
    {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let pantry_response = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([{
                            "name": "Milk",
                            "status": "out",
                            "quantityNote": "",
                            "autoAddToShoppingListWhenOut": true
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let pantry_response = response_json(pantry_response).await;
        let pantry_id = pantry_response[0]["id"].as_str().unwrap();
        let pantry_canonical_id = pantry_response[0]["canonicalIngredientId"]
            .as_str()
            .unwrap();
        let pantry_version = pantry_response[0]["version"].as_i64().unwrap();
        let list = app
            .clone()
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list = response_json(list).await;
        let automatic_id = list[0]["id"].as_str().unwrap();
        let kept = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/{automatic_id}/purchase"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header(
                        "idempotency-key",
                        format!("{}.automatic-keep", Utc::now().timestamp()),
                    )
                    .body(Body::from(
                        json!({
                            "disposition": "keep",
                            "shoppingExpectedVersion": list[0]["version"],
                            "pantryItemId": pantry_id,
                            "pantryExpectedVersion": pantry_version,
                            "expiresOn": null,
                            "quantityNote": "",
                            "autoAddToShoppingListWhenOut": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(kept.status(), StatusCode::OK);
        let kept = response_json(kept).await;
        assert_eq!(kept["pantry"]["status"], "have");
        assert_eq!(kept["shoppingItems"].as_array().unwrap().len(), 1);
        assert_eq!(kept["shoppingItems"][0]["sources"][0]["kind"], "manual");
        let keep_action = kept["purchaseActionId"].as_str().unwrap();
        let undo_keep = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/purchases/{keep_action}/undo"))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(undo_keep.status(), StatusCode::OK);
        let undo_keep = response_json(undo_keep).await;
        assert_eq!(undo_keep.as_array().unwrap().len(), 1);
        assert!(undo_keep[0]["id"].as_str().unwrap().starts_with("auto:"));

        let manual_milk = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "name": "Milk",
                            "canonicalIngredientId": pantry_canonical_id
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(manual_milk.status(), StatusCode::CREATED);
        let stale_keep = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/{automatic_id}/purchase"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header(
                        "idempotency-key",
                        format!("{}.stale-automatic-keep", Utc::now().timestamp()),
                    )
                    .body(Body::from(
                        json!({
                            "disposition": "keep",
                            "shoppingExpectedVersion": list[0]["version"],
                            "pantryItemId": pantry_id,
                            "pantryExpectedVersion": pantry_version,
                            "expiresOn": null,
                            "quantityNote": "",
                            "autoAddToShoppingListWhenOut": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale_keep.status(), StatusCode::OK);
        let stale_keep = response_json(stale_keep).await;
        let stale_action = stale_keep["purchaseActionId"].as_str().unwrap();
        let undo_stale = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/purchases/{stale_action}/undo"))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(undo_stale.status(), StatusCode::OK);
        let undo_stale = response_json(undo_stale).await;
        assert_eq!(undo_stale.as_array().unwrap().len(), 1);
        assert!(!undo_stale[0]["id"].as_str().unwrap().starts_with("auto:"));
        assert!(undo_stale[0]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["kind"] == "manual"));

        for name in ["Apple", "Banana"] {
            let added = app
                .clone()
                .oneshot(
                    Request::post("/api/shopping-list")
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .body(Body::from(json!({ "name": name }).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(added.status(), StatusCode::CREATED);
        }
        let list = app
            .clone()
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list = response_json(list).await;
        let apple = list
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Apple")
            .unwrap();
        let banana = list
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Banana")
            .unwrap();
        let shared_key = format!("{}.cross-item", Utc::now().timestamp());
        let shared_body = json!({
            "disposition": "remove",
            "shoppingExpectedVersion": 1,
            "createUsingCurrentName": true,
            "expiresOn": null,
            "quantityNote": "",
            "autoAddToShoppingListWhenOut": false
        })
        .to_string();
        let first = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/shopping-list/{}/purchase",
                    apple["id"].as_str().unwrap()
                ))
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .header("idempotency-key", &shared_key)
                .body(Body::from(shared_body.clone()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let cross_item = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/shopping-list/{}/purchase",
                    banana["id"].as_str().unwrap()
                ))
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .header("idempotency-key", &shared_key)
                .body(Body::from(shared_body))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cross_item.status(), StatusCode::CONFLICT);

        let apple_action = response_json(first).await["purchaseActionId"]
            .as_str()
            .unwrap()
            .to_string();
        let readded = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(json!({ "name": "Apple" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(readded.status(), StatusCode::CREATED);
        let undo_conflict = app
            .oneshot(
                Request::post(format!("/api/shopping-list/purchases/{apple_action}/undo"))
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(undo_conflict.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn shopping_items_dedupe_sources_and_validate_recipe_membership() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let add_milk = || {
            app.clone().oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(json!({"name": " Milk "}).to_string()))
                    .unwrap(),
            )
        };
        let (first, retry) = tokio::join!(add_milk(), add_milk());
        assert_eq!(first.unwrap().status(), StatusCode::CREATED);
        assert_eq!(retry.unwrap().status(), StatusCode::CREATED);
        let recipe = create_recipe(&app, &cookie, "private").await;
        let recipe_id = recipe["id"].as_str().unwrap();
        let added = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/shopping-list-items"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(json!({"name": "water"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(added.status(), StatusCode::CREATED);
        let invalid = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/shopping-list-items"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(json!({"name": "salt"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::NOT_FOUND);

        let list = app
            .clone()
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list = response_json(list).await;
        assert_eq!(list.as_array().unwrap().len(), 2);
        let milk = list
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Milk")
            .unwrap();
        assert_eq!(milk["sources"].as_array().unwrap().len(), 1);
        assert_eq!(milk["sources"][0]["kind"], "manual");
        let water = list
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "water")
            .unwrap();
        assert_eq!(water["sources"][0]["kind"], "recipe");
        assert_eq!(water["sources"][0]["recipeId"], recipe_id);
        let water_id = water["id"].as_str().unwrap();
        let water_version = water["version"].as_i64().unwrap();
        let partial = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/{water_id}/purchase"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header(
                        "idempotency-key",
                        format!("{}.recipe-partial", Utc::now().timestamp()),
                    )
                    .body(Body::from(
                        json!({
                            "disposition": "keep",
                            "shoppingExpectedVersion": water_version,
                            "createUsingCurrentName": true,
                            "expiresOn": "2026-11-07",
                            "quantityNote": "one pack",
                            "autoAddToShoppingListWhenOut": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(partial.status(), StatusCode::OK);
        let partial = response_json(partial).await;
        assert_eq!(partial["pantry"]["status"], "have");
        assert_eq!(partial["pantry"]["expiresOn"], "2026-11-07");
        assert_eq!(partial["pantry"]["quantityNote"], "one pack");
        let kept = partial["shoppingItems"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == water_id)
            .unwrap();
        assert_eq!(kept["sources"][0]["kind"], "recipe");
    }

    #[tokio::test]
    async fn pantry_dates_split_lists_and_preserve_expiry_when_omitted() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let invalid = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([{
                            "name": "Milk",
                            "status": "have",
                            "quantityNote": "",
                            "expiresOn": "2026-02-30"
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let created = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([{
                            "name": " Ｍｉｌｋ，milk、青菜\nRice ",
                            "status": "have",
                            "quantityNote": "fresh",
                            "expiresOn": "2026-11-04"
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let body = response_json(created).await;
        assert_eq!(body.as_array().unwrap().len(), 3);
        let milk = body
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Milk")
            .unwrap();
        assert_eq!(milk["expiresOn"], "2026-11-04");
        assert!(milk["canonicalIngredientId"].is_string());

        let recipe = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Milk glass",
                            "description": "",
                            "instructions": "",
                            "visibility": "private",
                            "ingredients": [{"name": "Milk", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(recipe.status(), StatusCode::CREATED);
        let recommendations = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({"pantryOnly": false, "today": "2026-11-04"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json(recommendations).await;
        let milk_recipe = body
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["title"] == "Milk glass")
            .unwrap();
        assert_eq!(milk_recipe["status"], "VERIFY");
        assert!(milk_recipe["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("expires today")));

        let status_only = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([{"name": "milk", "status": "unknown", "quantityNote": "check"}])
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json(status_only).await;
        let milk = body
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "milk")
            .unwrap();
        assert_eq!(milk["expiresOn"], "2026-11-04");

        let cleared = app
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!([{
                            "name": "milk",
                            "status": "have",
                            "quantityNote": "",
                            "expiresOn": null
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json(cleared).await;
        let milk = body
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "milk")
            .unwrap();
        assert!(milk["expiresOn"].is_null());
    }

    #[tokio::test]
    async fn ingredient_suggestions_respect_visibility_and_never_auto_apply_fuzzy_matches() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let other_cookie = login(&app, "Other").await;
        let pantry = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!([{
                            "name": "Tomato",
                            "status": "have",
                            "quantityNote": ""
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pantry.status(), StatusCode::OK);
        let pantry_body = response_json(pantry).await;
        let tomato_id = pantry_body[0]["canonicalIngredientId"]
            .as_str()
            .unwrap()
            .to_string();

        let owner_suggestions = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=tomoto")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json(owner_suggestions).await;
        assert_eq!(body[0]["displayName"], "Tomato");
        assert_eq!(body[0]["reason"], "similar");

        let other_suggestions = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=tomato")
                    .header("cookie", &other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(other_suggestions).await, json!([]));

        let selected_alias = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!([{
                            "name": "Roma tomato",
                            "status": "have",
                            "quantityNote": "",
                            "canonicalIngredientId": tomato_id
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(selected_alias.status(), StatusCode::OK);
        let alias_suggestions = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=roma%20tomato")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json(alias_suggestions).await;
        assert_eq!(body[0]["canonicalIngredientId"], tomato_id);
        assert_eq!(body[0]["reason"], "alias");

        let canonical_recipe = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({
                            "title": "Tomato plate",
                            "description": "",
                            "instructions": "",
                            "visibility": "private",
                            "ingredients": [{
                                "name": "Tomato",
                                "quantityNote": "",
                                "canonicalIngredientId": tomato_id
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(canonical_recipe.status(), StatusCode::CREATED);

        let recipe = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!({
                            "title": "Powder test",
                            "description": "",
                            "instructions": "",
                            "visibility": "private",
                            "ingredients": [{"name": "Baking powder", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(recipe.status(), StatusCode::CREATED);
        let baking_soda = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!([{"name": "Baking soda", "status": "have", "quantityNote": ""}])
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(baking_soda.status(), StatusCode::OK);
        let recommendations = app
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", owner_cookie)
                    .body(Body::from(json!({"pantryOnly": true}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json(recommendations).await;
        assert!(body
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["recipe"]["title"] != "Powder test"));
        let tomato = body
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["title"] == "Tomato plate")
            .unwrap();
        assert_eq!(tomato["status"], "READY");
    }

    #[tokio::test]
    async fn ingredient_suggestions_keep_cjk_terms_distinct_and_aliases_explicit() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "CJK owner").await;
        let pantry = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([
                            {"name": "酱油", "status": "have", "quantityNote": ""},
                            {"name": "油", "status": "have", "quantityNote": ""},
                            {"name": "鸡腿", "status": "have", "quantityNote": ""},
                            {"name": "鸡翅", "status": "have", "quantityNote": ""},
                            {"name": "鸡胸肉", "status": "have", "quantityNote": ""},
                            {"name": "Soy Sauce", "status": "have", "quantityNote": ""},
                            {"name": "Tomato", "status": "have", "quantityNote": ""},
                            {"name": "baton", "status": "have", "quantityNote": ""},
                            {"name": "caton", "status": "have", "quantityNote": ""},
                            {"name": "haton", "status": "have", "quantityNote": ""}
                        ])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pantry.status(), StatusCode::OK);
        let pantry = response_json(pantry).await;
        let id_for = |name: &str| {
            pantry
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["name"] == name)
                .unwrap()["canonicalIngredientId"]
                .as_str()
                .unwrap()
                .to_string()
        };

        for (query, expected) in [
            ("%E9%85%B1%E6%B2%B9", "酱油"),
            ("%E6%B2%B9", "油"),
            ("%E9%B8%A1%E8%85%BF", "鸡腿"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!("/api/ingredients/suggestions?q={query}"))
                        .header("cookie", &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let response = response_json(response).await;
            assert_eq!(response.as_array().unwrap().len(), 1);
            assert_eq!(response[0]["displayName"], expected);
            assert_eq!(response[0]["reason"], "exact");
        }
        let single_han = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=%E9%B8%A1")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(single_han).await, json!([]));

        let normalized_exact = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=%20%EF%BC%B3%EF%BC%AF%EF%BC%B9%E3%80%80%EF%BC%B3%EF%BC%A1%EF%BC%B5%EF%BC%A3%EF%BC%A5%20")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let normalized_exact = response_json(normalized_exact).await;
        assert_eq!(normalized_exact[0]["displayName"], "Soy Sauce");
        assert_eq!(normalized_exact[0]["reason"], "exact");

        let fuzzy = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=maton")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let fuzzy = response_json(fuzzy).await;
        assert_eq!(fuzzy.as_array().unwrap().len(), 2);
        assert!(fuzzy
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["reason"] == "similar"));

        let fuzzy_selection = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([{
                            "name": "Tomoto",
                            "status": "have",
                            "quantityNote": "",
                            "canonicalIngredientId": id_for("Tomato")
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(fuzzy_selection.status(), StatusCode::OK);
        let still_advisory = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=tomoto")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(still_advisory).await[0]["reason"], "similar");

        let confirmed_alias = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([{
                            "name": "鸡胸脯肉",
                            "status": "have",
                            "quantityNote": "",
                            "canonicalIngredientId": id_for("鸡胸肉")
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(confirmed_alias.status(), StatusCode::OK);
        let alias = app
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=%E9%B8%A1%E8%83%B8%E8%84%AF%E8%82%89")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let alias = response_json(alias).await;
        assert_eq!(alias.as_array().unwrap().len(), 1);
        assert_eq!(alias[0]["displayName"], "鸡胸肉");
        assert_eq!(alias[0]["reason"], "alias");
    }

    #[tokio::test]
    async fn separate_cjk_ingredients_survive_recipe_recommend_shop_and_purchase() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "CJK lifecycle").await;
        let created = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "双油菜",
                            "description": "",
                            "instructions": "Cook.",
                            "visibility": "private",
                            "ingredients": [
                                {"name": "酱油", "quantityNote": "1 tbsp"},
                                {"name": "油", "quantityNote": "1 tbsp"}
                            ]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let recipe = response_json(created).await;
        let recipe_id = recipe["id"].as_str().unwrap().to_string();
        let ingredients = recipe["ingredients"].as_array().unwrap();
        assert_eq!(ingredients.len(), 2);
        assert_ne!(
            ingredients[0]["canonicalIngredientId"],
            ingredients[1]["canonicalIngredientId"]
        );
        let oil_canonical_id = ingredients
            .iter()
            .find(|ingredient| ingredient["name"] == "油")
            .unwrap()["canonicalIngredientId"]
            .clone();

        let soy_only = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Only soy",
                            "description": "",
                            "instructions": "Cook.",
                            "visibility": "private",
                            "ingredients": [{"name": "酱油", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(soy_only.status(), StatusCode::CREATED);
        let soy_only_id = response_json(soy_only).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let oil_required = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "required": [{
                                "name": "油",
                                "canonicalIngredientId": oil_canonical_id
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let oil_required = response_json(oil_required).await;
        assert!(oil_required
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["recipe"]["id"] == recipe_id));
        assert!(!oil_required
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["recipe"]["id"] == soy_only_id));

        for ingredient in ingredients {
            let added = app
                .clone()
                .oneshot(
                    Request::post(format!("/api/recipes/{recipe_id}/shopping-list-items"))
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .body(Body::from(
                            json!({
                                "name": ingredient["name"],
                                "canonicalIngredientId": ingredient["canonicalIngredientId"]
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!([StatusCode::OK, StatusCode::CREATED].contains(&added.status()));
        }
        let shopping = app
            .clone()
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let shopping = response_json(shopping).await;
        assert_eq!(shopping.as_array().unwrap().len(), 2);
        assert!(shopping
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "酱油"));
        assert!(shopping
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "油"));

        for (index, item) in shopping.as_array().unwrap().iter().enumerate() {
            let purchased = app
                .clone()
                .oneshot(
                    Request::post(format!(
                        "/api/shopping-list/{}/purchase",
                        item["id"].as_str().unwrap()
                    ))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header(
                        "idempotency-key",
                        format!("{}.cjk-{index}", Utc::now().timestamp()),
                    )
                    .body(Body::from(
                        json!({
                            "disposition": "remove",
                            "shoppingExpectedVersion": item["version"],
                            "quantityNote": "",
                            "autoAddToShoppingListWhenOut": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(purchased.status(), StatusCode::OK);
        }
        let pantry = app
            .clone()
            .oneshot(
                Request::get("/api/pantry")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let pantry = response_json(pantry).await;
        let soy = pantry
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "酱油")
            .unwrap();
        let oil = pantry
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "油")
            .unwrap();
        assert_ne!(soy["canonicalIngredientId"], oil["canonicalIngredientId"]);

        let recommendations = app
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let recommendations = response_json(recommendations).await;
        let lifecycle = recommendations
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == recipe_id)
            .unwrap();
        assert_eq!(lifecycle["status"], "READY");
        assert_eq!(lifecycle["available"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn recommendation_filters_preserve_unresolved_identity_through_api_route() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Recommendation identity").await;
        let soy = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Canonical soy",
                            "description": "",
                            "instructions": "Cook.",
                            "visibility": "private",
                            "ingredients": [{"name": "酱油", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(soy.status(), StatusCode::CREATED);
        let soy = response_json(soy).await;
        let soy_id = soy["id"].as_str().unwrap();
        let soy_canonical_id = soy["ingredients"][0]["canonicalIngredientId"]
            .as_str()
            .unwrap();

        let oil = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Canonical oil",
                            "description": "",
                            "instructions": "Cook.",
                            "visibility": "private",
                            "ingredients": [{"name": "油", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(oil.status(), StatusCode::CREATED);
        let oil = response_json(oil).await;
        let oil_id = oil["id"].as_str().unwrap();
        let oil_canonical_id = oil["ingredients"][0]["canonicalIngredientId"]
            .as_str()
            .unwrap();

        for preferred in [
            json!(["酱油"]),
            json!([{"name": "酱油", "canonicalIngredientId": null}]),
            json!(["油"]),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/api/recommendations")
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .body(Body::from(json!({"preferred": preferred}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = response_json(response).await;
            for recipe_id in [soy_id, oil_id] {
                let result = body
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|item| item["recipe"]["id"] == recipe_id)
                    .unwrap();
                assert_eq!(result["desiredMatches"], json!([]));
            }
        }

        for required in [
            json!(["酱油"]),
            json!([{"name": "酱油", "canonicalIngredientId": null}]),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/api/recommendations")
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .body(Body::from(json!({"required": required}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = response_json(response).await;
            assert!(body
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["recipe"]["id"] != soy_id));
        }

        for excluded in [
            json!(["酱油"]),
            json!([{"name": "酱油", "canonicalIngredientId": null}]),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/api/recommendations")
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .body(Body::from(json!({"excluded": excluded}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = response_json(response).await;
            assert!(body
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["recipe"]["id"] == soy_id));
        }

        let explicit_preferred = app
            .clone()
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "preferred": [{
                                "name": "油",
                                "canonicalIngredientId": oil_canonical_id
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let explicit_preferred = response_json(explicit_preferred).await;
        let explicit_oil = explicit_preferred
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == oil_id)
            .unwrap();
        assert_eq!(explicit_oil["desiredMatches"], json!(["油"]));
        let explicit_soy = explicit_preferred
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == soy_id)
            .unwrap();
        assert_eq!(explicit_soy["desiredMatches"], json!([]));

        let explicit_hard_filters = app
            .oneshot(
                Request::post("/api/recommendations")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!({
                            "preferred": [{
                                "name": "油",
                                "canonicalIngredientId": oil_canonical_id
                            }],
                            "required": [{
                                "name": "酱油",
                                "canonicalIngredientId": soy_canonical_id
                            }],
                            "excluded": [{
                                "name": "油",
                                "canonicalIngredientId": oil_canonical_id
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let explicit_hard_filters = response_json(explicit_hard_filters).await;
        let explicit_soy = explicit_hard_filters
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["id"] == soy_id)
            .unwrap();
        assert_eq!(explicit_soy["desiredMatches"], json!([]));
        assert!(explicit_hard_filters
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["recipe"]["id"] != oil_id));
    }

    #[tokio::test]
    async fn shopping_does_not_offer_contained_cjk_terms_as_purchase_candidates() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "CJK shopping").await;
        let pantry = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([{"name": "油", "status": "have", "quantityNote": ""}]).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pantry.status(), StatusCode::OK);
        let shopping = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(json!({"name": "酱油"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(shopping.status(), StatusCode::CREATED);
        let shopping = response_json(shopping).await;
        assert_eq!(shopping[0]["purchaseOptions"]["resolution"], "create");
        assert_eq!(shopping[0]["purchaseOptions"]["candidates"], json!([]));
        let purchased = app
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/shopping-list/{}/purchase",
                    shopping[0]["id"].as_str().unwrap()
                ))
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .header(
                    "idempotency-key",
                    format!("{}.cjk-contained", Utc::now().timestamp()),
                )
                .body(Body::from(
                    json!({
                        "disposition": "remove",
                        "shoppingExpectedVersion": shopping[0]["version"],
                        "quantityNote": "",
                        "autoAddToShoppingListWhenOut": false
                    })
                    .to_string(),
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(purchased.status(), StatusCode::OK);
        let pantry = app
            .oneshot(
                Request::get("/api/pantry")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let pantry = response_json(pantry).await;
        assert_eq!(pantry.as_array().unwrap().len(), 2);
        assert!(pantry
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "油"));
        assert!(pantry
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "酱油"));
    }

    #[tokio::test]
    async fn ingredient_suggestions_prefer_exact_matches_and_preserve_ambiguity() {
        let (app, _, pool) = app_with_runtime().await;
        let owner_cookie = login(&app, "Exact owner").await;
        let owner_id: String =
            sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'Exact owner'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let pantry = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!([
                            {"name": "Tomato", "status": "have", "quantityNote": ""},
                            {"name": "Tomato sauce", "status": "have", "quantityNote": ""},
                            {"name": "Plum tomato", "status": "have", "quantityNote": ""},
                            {"name": "Roma tomato sauce", "status": "have", "quantityNote": ""}
                        ])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pantry.status(), StatusCode::OK);
        let pantry = response_json(pantry).await;
        let tomato_id = pantry
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Tomato")
            .unwrap()["canonicalIngredientId"]
            .as_str()
            .unwrap()
            .to_string();
        let plum_tomato_id = pantry
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "Plum tomato")
            .unwrap()["canonicalIngredientId"]
            .as_str()
            .unwrap()
            .to_string();

        let exact = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=%EF%BC%B4%EF%BD%8F%EF%BD%8D%EF%BD%81%EF%BD%94%EF%BD%8F")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response_json(exact).await,
            json!([{
                "canonicalIngredientId": tomato_id,
                "displayName": "Tomato",
                "matchedText": "Tomato",
                "reason": "exact"
            }])
        );

        sqlx::query(
            "INSERT INTO ingredient_aliases
             (id, owner_id, alias_name, alias_normalized, canonical_ingredient_id, created_at)
             VALUES (?, ?, 'Roma tomato', 'roma tomato', ?, unixepoch())",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&owner_id)
        .bind(&plum_tomato_id)
        .execute(&pool)
        .await
        .unwrap();
        let alias = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=roma%20tomato")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let alias = response_json(alias).await;
        assert_eq!(alias.as_array().unwrap().len(), 1);
        assert_eq!(alias[0]["canonicalIngredientId"], plum_tomato_id);
        assert_eq!(alias[0]["reason"], "alias");

        sqlx::query(
            "INSERT INTO ingredient_aliases
             (id, owner_id, alias_name, alias_normalized, canonical_ingredient_id, created_at)
             VALUES (?, ?, 'Tomato', 'tomato', ?, unixepoch())",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&owner_id)
        .bind(&plum_tomato_id)
        .execute(&pool)
        .await
        .unwrap();
        let ambiguous = app
            .clone()
            .oneshot(
                Request::get("/api/ingredients/suggestions?q=tomato")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let ambiguous = response_json(ambiguous).await;
        assert_eq!(ambiguous.as_array().unwrap().len(), 2);
        assert!(ambiguous
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["reason"] == "exact" || item["reason"] == "alias"));
        let unsafe_implicit_resolution = app
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", owner_cookie)
                    .body(Body::from(
                        json!({
                            "title": "Ambiguous tomato",
                            "description": "",
                            "instructions": "Cook.",
                            "visibility": "private",
                            "ingredients": [{"name": "Tomato", "quantityNote": ""}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unsafe_implicit_resolution.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn logout_revokes_the_server_side_session() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let logout = app
            .clone()
            .oneshot(
                Request::post("/api/auth/logout")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);
        let me = app
            .oneshot(
                Request::get("/api/me")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn shopping_choice_purchase_applies_only_selected_alternative_and_undo_restores_choice() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let created = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Flexible toast",
                            "description": "",
                            "instructions": "Toast.",
                            "visibility": "private",
                            "ingredientModel": "groups-v1",
                            "ingredientGroups": [{
                                "id": "spread",
                                "optional": false,
                                "note": "",
                                "sortOrder": 0,
                                "alternatives": [
                                    {"id": "butter", "name": "butter", "quantityNote": "", "sortOrder": 0},
                                    {"id": "oil", "name": "olive oil", "quantityNote": "", "sortOrder": 1}
                                ]
                            }]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let recipe = response_json(created).await;
        let recipe_id = recipe["id"].as_str().unwrap();
        let added = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/shopping-list-items"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(json!({"groupId": "spread"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(added.status(), StatusCode::CREATED);
        let items = response_json(added).await;
        assert_eq!(
            items[0]["purchaseOptions"]["resolution"],
            "selectAlternative"
        );
        assert_eq!(items[0]["choiceAlternatives"].as_array().unwrap().len(), 2);
        let item_id = items[0]["id"].as_str().unwrap();
        let version = items[0]["version"].as_i64().unwrap();
        let purchase_key = format!("{}.choice-purchase", Utc::now().timestamp());
        let purchased = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/{item_id}/purchase"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("idempotency-key", purchase_key)
                    .body(Body::from(
                        json!({
                            "disposition": "remove",
                            "shoppingExpectedVersion": version,
                            "selectedAlternativeId": "oil",
                            "createUsingCurrentName": true,
                            "quantityNote": "",
                            "autoAddToShoppingListWhenOut": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(purchased.status(), StatusCode::OK);
        let purchased = response_json(purchased).await;
        assert_eq!(purchased["pantry"]["name"], "olive oil");
        assert!(purchased["shoppingItems"].as_array().unwrap().is_empty());
        let action_id = purchased["purchaseActionId"].as_str().unwrap();
        let undone = app
            .oneshot(
                Request::post(format!("/api/shopping-list/purchases/{action_id}/undo"))
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(undone.status(), StatusCode::OK);
        let restored = response_json(undone).await;
        assert_eq!(
            restored[0]["choiceAlternatives"].as_array().unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn recipe_update_requires_valid_explicit_group_and_alternative_sort_orders() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Ordered recipe editor").await;
        let groups = json!([
            {
                "id": "egg",
                "optional": false,
                "note": "",
                "sortOrder": 0,
                "alternatives": [
                    {"id": "chicken", "name": "Chicken egg", "quantityNote": "2", "sortOrder": 0},
                    {"id": "duck", "name": "Duck egg", "quantityNote": "2", "sortOrder": 1}
                ]
            },
            {
                "id": "water",
                "optional": false,
                "note": "",
                "sortOrder": 1,
                "alternatives": [
                    {"id": "water-primary", "name": "Water", "quantityNote": "200 ml", "sortOrder": 0}
                ]
            }
        ]);
        let created = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Steamed egg",
                            "description": "",
                            "instructions": "Steam.",
                            "visibility": "public",
                            "ingredientModel": "groups-v1",
                            "ingredientGroups": groups
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = response_json(created).await;
        let recipe_id = created["id"].as_str().unwrap();

        let updated = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .header("if-match", "\"recipe-v1\"")
                    .body(Body::from(
                        json!({
                            "title": "Steamed egg with alternatives",
                            "description": "",
                            "instructions": "Steam.",
                            "visibility": "public",
                            "ingredientModel": "groups-v1",
                            "ingredientGroups": groups
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::OK);
        let updated = response_json(updated).await;
        assert_eq!(updated["version"], 2);
        assert_eq!(updated["ingredientGroups"][0]["sortOrder"], 0);
        assert_eq!(
            updated["ingredientGroups"][0]["alternatives"][1]["sortOrder"],
            1
        );
        assert_eq!(updated["ingredientGroups"][1]["sortOrder"], 1);

        let mut invalid_groups = Vec::new();
        let mut duplicate_group = groups.clone();
        duplicate_group[1]["sortOrder"] = json!(0);
        invalid_groups.push(duplicate_group);
        let mut negative_group = groups.clone();
        negative_group[0]["sortOrder"] = json!(-1);
        invalid_groups.push(negative_group);
        let mut duplicate_alternative = groups.clone();
        duplicate_alternative[0]["alternatives"][1]["sortOrder"] = json!(0);
        invalid_groups.push(duplicate_alternative);
        let mut negative_alternative = groups;
        negative_alternative[0]["alternatives"][0]["sortOrder"] = json!(-1);
        invalid_groups.push(negative_alternative);

        for invalid in invalid_groups {
            let response = app
                .clone()
                .oneshot(
                    Request::put(format!("/api/recipes/{recipe_id}"))
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .header("if-match", "\"recipe-v2\"")
                        .body(Body::from(
                            json!({
                                "title": "Invalid order",
                                "description": "",
                                "instructions": "Steam.",
                                "visibility": "public",
                                "ingredientModel": "groups-v1",
                                "ingredientGroups": invalid
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response_json(response).await["error"], "validation_error");
        }

        let mut missing_group_order = json!([
            {
                "id": "egg",
                "optional": false,
                "note": "",
                "sortOrder": 0,
                "alternatives": [
                    {"id": "chicken", "name": "Chicken egg", "quantityNote": "2", "sortOrder": 0}
                ]
            }
        ]);
        missing_group_order[0]
            .as_object_mut()
            .unwrap()
            .remove("sortOrder");
        let missing = app
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .header("if-match", "\"recipe-v2\"")
                    .body(Body::from(
                        json!({
                            "title": "Missing order",
                            "description": "",
                            "instructions": "Steam.",
                            "visibility": "public",
                            "ingredientModel": "groups-v1",
                            "ingredientGroups": missing_group_order
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn shopping_delete_is_versioned_idempotent_owner_scoped_and_leaves_pantry_and_recipes() {
        let app = build_app(Config::test()).await.unwrap();
        let owner_cookie = login(&app, "Owner").await;
        let other_cookie = login(&app, "Other").await;
        let pantry = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(
                        json!([{"name": "milk", "status": "have", "quantityNote": "1 carton"}])
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let pantry_before = response_json(pantry).await;

        let manual = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(json!({"name": "salt"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let manual = response_json(manual).await;
        let manual_id = manual[0]["id"].as_str().unwrap();
        let manual_version = manual[0]["version"].as_i64().unwrap();
        let key = format!("{}.delete-manual", Utc::now().timestamp());
        let first =
            delete_shopping_item(&app, &owner_cookie, manual_id, manual_version, &key).await;
        assert_eq!(first.status(), StatusCode::OK);
        let first_body = response_json(first).await;
        assert_eq!(first_body["outcome"], "removed");
        assert_eq!(first_body["undoAvailable"], false);
        for _ in 0..2 {
            let replay =
                delete_shopping_item(&app, &owner_cookie, manual_id, manual_version, &key).await;
            assert_eq!(replay.status(), StatusCode::OK);
            assert_eq!(response_json(replay).await, first_body);
        }

        let pantry_after = app
            .clone()
            .oneshot(
                Request::get("/api/pantry")
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(pantry_after).await, pantry_before);

        let recipe = create_recipe(&app, &owner_cookie, "private").await;
        let recipe_id = recipe["id"].as_str().unwrap();
        let recipe_item = app
            .clone()
            .oneshot(
                Request::post(format!("/api/recipes/{recipe_id}/shopping-list-items"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(json!({"name": "water"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let recipe_item = response_json(recipe_item).await;
        let recipe_item_id = recipe_item[0]["id"].as_str().unwrap();
        let recipe_item_version = recipe_item[0]["version"].as_i64().unwrap();
        let deleted_recipe_source = delete_shopping_item(
            &app,
            &owner_cookie,
            recipe_item_id,
            recipe_item_version,
            &format!("{}.delete-recipe", Utc::now().timestamp()),
        )
        .await;
        assert_eq!(deleted_recipe_source.status(), StatusCode::OK);
        let recipe_after = app
            .clone()
            .oneshot(
                Request::get(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response_json(recipe_after).await["ingredients"],
            recipe["ingredients"]
        );

        let stale = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(json!({"name": "pepper"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let stale = response_json(stale).await;
        let stale_id = stale[0]["id"].as_str().unwrap();
        let stale_version = stale[0]["version"].as_i64().unwrap();
        let bumped = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
                    .body(Body::from(json!({"name": "pepper"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bumped.status(), StatusCode::CREATED);
        let conflict = delete_shopping_item(
            &app,
            &owner_cookie,
            stale_id,
            stale_version,
            &format!("{}.delete-stale", Utc::now().timestamp()),
        )
        .await;
        assert_eq!(conflict.status(), StatusCode::CONFLICT);

        let other = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &other_cookie)
                    .body(Body::from(json!({"name": "private spice"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let other = response_json(other).await;
        let cross_account = delete_shopping_item(
            &app,
            &owner_cookie,
            other[0]["id"].as_str().unwrap(),
            other[0]["version"].as_i64().unwrap(),
            &format!("{}.delete-cross-account", Utc::now().timestamp()),
        )
        .await;
        assert_eq!(cross_account.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn shopping_delete_handles_automatic_only_and_merged_without_mutating_pantry() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let pantry = app
            .clone()
            .oneshot(
                Request::put("/api/pantry")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!([{
                            "name": "milk",
                            "status": "out",
                            "quantityNote": "1 carton",
                            "autoAddToShoppingListWhenOut": true
                        }])
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let pantry_before = response_json(pantry).await;
        let canonical_id = pantry_before[0]["canonicalIngredientId"].as_str().unwrap();
        let automatic_id = format!("auto:{}", pantry_before[0]["id"].as_str().unwrap());
        let automatic_version = pantry_before[0]["version"].as_i64().unwrap();
        let automatic = delete_shopping_item(
            &app,
            &cookie,
            &automatic_id,
            automatic_version,
            &format!("{}.delete-automatic", Utc::now().timestamp()),
        )
        .await;
        assert_eq!(automatic.status(), StatusCode::OK);
        let automatic = response_json(automatic).await;
        assert_eq!(automatic["outcome"], "automaticOnly");
        assert_eq!(automatic["requiresDisableAutoRestock"], true);
        assert_eq!(automatic["shoppingItems"][0]["id"], automatic_id);

        let merged = app
            .clone()
            .oneshot(
                Request::post("/api/shopping-list")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({"name": "milk", "canonicalIngredientId": canonical_id}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let merged = response_json(merged).await;
        let persisted = merged
            .as_array()
            .unwrap()
            .iter()
            .find(|item| !item["id"].as_str().unwrap().starts_with("auto:"))
            .unwrap();
        assert!(persisted["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["kind"] == "automatic"));
        let deleted = delete_shopping_item(
            &app,
            &cookie,
            persisted["id"].as_str().unwrap(),
            persisted["version"].as_i64().unwrap(),
            &format!("{}.delete-merged", Utc::now().timestamp()),
        )
        .await;
        assert_eq!(deleted.status(), StatusCode::OK);
        let deleted = response_json(deleted).await;
        assert_eq!(deleted["outcome"], "automaticRemains");
        assert_eq!(deleted["requiresDisableAutoRestock"], true);
        assert_eq!(deleted["shoppingItems"][0]["id"], automatic_id);
        let pantry_after = app
            .oneshot(
                Request::get("/api/pantry")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response_json(pantry_after).await, pantry_before);
    }

    #[tokio::test]
    async fn shopping_choice_delete_removes_only_the_exact_choice_item() {
        let app = build_app(Config::test()).await.unwrap();
        let cookie = login(&app, "Owner").await;
        let created = app
            .clone()
            .oneshot(
                Request::post("/api/recipes")
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        json!({
                            "title": "Two choices",
                            "description": "",
                            "instructions": "Cook.",
                            "visibility": "private",
                            "ingredientModel": "groups-v1",
                            "ingredientGroups": [
                                {
                                    "id": "fat",
                                    "optional": false,
                                    "note": "",
                                    "sortOrder": 0,
                                    "alternatives": [
                                        {"id": "butter", "name": "butter", "quantityNote": "", "sortOrder": 0},
                                        {"id": "oil", "name": "oil", "quantityNote": "", "sortOrder": 1}
                                    ]
                                },
                                {
                                    "id": "herb",
                                    "optional": false,
                                    "note": "",
                                    "sortOrder": 1,
                                    "alternatives": [
                                        {"id": "basil", "name": "basil", "quantityNote": "", "sortOrder": 0},
                                        {"id": "parsley", "name": "parsley", "quantityNote": "", "sortOrder": 1}
                                    ]
                                }
                            ]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let recipe = response_json(created).await;
        let recipe_id = recipe["id"].as_str().unwrap();
        for group_id in ["fat", "herb"] {
            let added = app
                .clone()
                .oneshot(
                    Request::post(format!("/api/recipes/{recipe_id}/shopping-list-items"))
                        .header("content-type", "application/json")
                        .header("cookie", &cookie)
                        .body(Body::from(json!({"groupId": group_id}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(added.status(), StatusCode::CREATED);
        }
        let listed = app
            .clone()
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let listed = response_json(listed).await;
        assert_eq!(listed.as_array().unwrap().len(), 2);
        let fat = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "butter / oil")
            .unwrap();
        let deleted = delete_shopping_item(
            &app,
            &cookie,
            fat["id"].as_str().unwrap(),
            fat["version"].as_i64().unwrap(),
            &format!("{}.delete-choice", Utc::now().timestamp()),
        )
        .await;
        assert_eq!(deleted.status(), StatusCode::OK);
        let deleted = response_json(deleted).await;
        assert_eq!(deleted["outcome"], "removed");
        assert_eq!(deleted["shoppingItems"].as_array().unwrap().len(), 1);
        assert_eq!(deleted["shoppingItems"][0]["name"], "basil / parsley");
        let recipe_after = app
            .oneshot(
                Request::get(format!("/api/recipes/{recipe_id}"))
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response_json(recipe_after).await["ingredientGroups"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn logout_all_revokes_every_session_for_the_user() {
        let app = build_app(Config::test()).await.unwrap();
        let first = login(&app, "Owner").await;
        let second = login(&app, "Owner").await;
        let logout = app
            .clone()
            .oneshot(
                Request::post("/api/auth/logout-all")
                    .header("cookie", &first)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);
        for cookie in [first, second] {
            let me = app
                .clone()
                .oneshot(
                    Request::get("/api/me")
                        .header("cookie", cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn production_google_start_uses_host_cookie_and_backend_redirect() {
        let mut config = Config::test();
        config.environment = AppEnvironment::Production;
        config.auth_mode = AuthMode::Google;
        config.public_origin = "https://cook.vichare.net".into();
        config.oauth_state_digest_key = [1; 32];
        config.session_digest_key = [2; 32];
        config.email_otp_digest_key = [3; 32];
        config.passkey_ceremony_digest_key = [4; 32];
        config.google_client_id = Some("client".into());
        config.google_client_secret = Some("secret".into());
        config.smtp = Some(crate::config::SmtpConfig {
            host: "smtp.example.test".into(),
            port: 465,
            tls_mode: crate::config::SmtpTlsMode::Tls,
            username: "user".into(),
            password: "password".into(),
            from: "Cook <cook@example.test>".into(),
        });
        config.passkey_rp_id = "cook.vichare.net".into();
        config.passkey_rp_origin = "https://cook.vichare.net".into();
        let app = build_app(config).await.unwrap();
        let response = app
            .oneshot(
                Request::get("/api/auth/google/start")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        let location = response
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(location.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(location.contains("code_challenge_method=S256"));
        let cookie = response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.starts_with("__Host-cook_oauth_state="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(!cookie.contains("Domain="));
    }
}

use axum::{
    extract::{Path, Query, State},
    http::{header::SET_COOKIE, HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{NaiveDate, Utc};
use rust_web_auth_core::{AuthenticatedContext, LoginCallback, LoginError, UserId};
use rust_web_auth_email_otp::{EmailOtpRequest, EmailOtpVerification};
use rust_web_auth_http::{expired_cookie, issued_cookie};
use rust_web_auth_passkey::{
    KnownUserRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    ai_recipes,
    auth::{load_user, upsert_development_user, GoogleCompletionError},
    config::AuthMode,
    error::{AppError, AppResult},
    images, imports,
    ingredients::{
        clean_display_name, match_reason, normalize_ingredient, split_ingredient_list, MatchReason,
    },
    models::{
        normalize_nickname, IngredientInput, LocaleInput, PantryItem, PantryItemInput,
        PantryStatus, ProfileInput, Recipe, RecipeImage, RecipeInput, RecipeSource,
        RecommendationRequest, RecommendationSettings, User,
    },
    passwordless::{
        email_address, map_email_error, map_passkey_error, normalize_email, purpose,
        PasskeySummary, EMAIL_LINK_PURPOSE, EMAIL_SIGN_IN_PURPOSE,
    },
    recommendation::{recommend, Recommendation},
    shopping, AppState,
};

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
            put(shopping::update_manual_item).delete(shopping::delete_manual_item),
        )
        .route(
            "/api/shopping-list/{id}/complete",
            post(shopping::complete_manual_item),
        )
        .route(
            "/api/shopping-list/automatic/{id}/purchased",
            post(shopping::mark_automatic_purchased),
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
struct CurrentAuth(AuthenticatedContext);

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
    issue_user_session(&state, user_id).await
}

async fn link_email_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(input): Json<EmailOtpRequestInput>,
) -> AppResult<Json<EmailOtpChallengeResponse>> {
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
    Json(input): Json<EmailOtpVerifyInput>,
) -> AppResult<Json<SecuritySettings>> {
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
        .linked_emails(&user.id)
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
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
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
    let user = load_user(&state.pool, &context.user_id).await?;
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
    Path(id): Path<String>,
    Json(mut input): Json<RecipeInput>,
) -> AppResult<Json<Recipe>> {
    input.ingredients = expand_ingredient_inputs(input.ingredients);
    validate_recipe(&input)?;
    let mut transaction = state.pool.begin().await?;
    let previous_visibility: String =
        sqlx::query_scalar("SELECT visibility FROM recipes WHERE id = ? AND owner_id = ?")
            .bind(&id)
            .bind(&user.id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(AppError::NotFound)?;
    let result = sqlx::query(
        "UPDATE recipes SET title = ?, description = ?, instructions = ?, visibility = ?,
         version = version + 1, updated_at = CURRENT_TIMESTAMP
         WHERE id = ? AND owner_id = ?",
    )
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(input.instructions.trim())
    .bind(&input.visibility)
    .bind(&id)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    replace_ingredients(&mut transaction, &id, &user.id, &input.ingredients).await?;
    if previous_visibility == "public" && input.visibility == "private" {
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

async fn delete_recipe(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let result = sqlx::query("DELETE FROM recipes WHERE id = ? AND owner_id = ?")
        .bind(id)
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
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
               AND (r.owner_id = ? OR r.visibility = 'public')
         ) OR EXISTS (
             SELECT 1 FROM ingredient_aliases a
             WHERE a.canonical_ingredient_id = c.id AND a.owner_id = ?
         )",
    )
    .bind(&user.id)
    .bind(&user.id)
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
    let aliases: Vec<(String, String)> = sqlx::query_as(
        "SELECT canonical_ingredient_id, alias_name
         FROM ingredient_aliases WHERE owner_id = ?",
    )
    .bind(&user.id)
    .fetch_all(&state.pool)
    .await?;
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
    suggestions.sort_by(|left, right| {
        left.reason
            .cmp(&right.reason)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    suggestions.truncate(8);
    Ok(Json(suggestions))
}

async fn list_pantry(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Vec<PantryItem>>> {
    Ok(Json(load_pantry(&state.pool, &user.id).await?))
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
         (owner_id, include_favorites, include_own, include_public, verification_days)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(owner_id) DO UPDATE SET include_favorites = excluded.include_favorites,
           include_own = excluded.include_own, include_public = excluded.include_public,
           verification_days = excluded.verification_days",
    )
    .bind(&user.id)
    .bind(settings.include_favorites)
    .bind(settings.include_own)
    .bind(settings.include_public)
    .bind(settings.verification_days)
    .execute(&state.pool)
    .await?;
    Ok(Json(settings))
}

async fn get_recommendations(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(mut request): Json<RecommendationRequest>,
) -> AppResult<Json<Vec<Recommendation>>> {
    let recommendation_time = if let Some(today) = request.today.as_deref() {
        parse_date_only(today, "today")?
            .and_hms_opt(12, 0, 0)
            .expect("noon is a valid time")
            .and_utc()
    } else {
        Utc::now()
    };
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
    resolve_recommendation_request(&state.pool, &user.id, &mut request).await?;
    let settings = load_settings(&state.pool, &user.id).await?;
    let candidates = load_candidate_recipes(&state.pool, &user.id, &settings).await?;
    let pantry = load_pantry(&state.pool, &user.id).await?;
    Ok(Json(recommend(
        candidates,
        &pantry,
        &request,
        settings.verification_days,
        recommendation_time,
    )))
}

fn seconds_until(expires_at: chrono::DateTime<Utc>) -> u64 {
    u64::try_from((expires_at - Utc::now()).num_seconds().max(0)).unwrap_or(0)
}

fn rate_limit_key(headers: &HeaderMap, key: &[u8; 32]) -> String {
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
    if input.ingredients.is_empty()
        || input
            .ingredients
            .iter()
            .any(|item| item.name.trim().is_empty())
    {
        return Err(AppError::Validation(
            "at least one named ingredient is required".into(),
        ));
    }
    if input.ingredients.len() > 100
        || input
            .ingredients
            .iter()
            .any(|item| item.name.chars().count() > 120 || item.quantity_note.chars().count() > 240)
    {
        return Err(AppError::Validation(
            "recipes may contain at most 100 ingredients with 120-character names and 240-character notes".into(),
        ));
    }
    if input.visibility != "private" && input.visibility != "public" {
        return Err(AppError::Validation(
            "visibility must be private or public".into(),
        ));
    }
    Ok(())
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
    } else if let Some(alias_id) = sqlx::query_scalar(
        "SELECT canonical_ingredient_id FROM ingredient_aliases
         WHERE owner_id = ? AND alias_normalized = ?",
    )
    .bind(owner_id)
    .bind(&normalized)
    .fetch_optional(&mut **transaction)
    .await?
    {
        alias_id
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
    };
    if selected_id.is_some() {
        let canonical_normalized: String =
            sqlx::query_scalar("SELECT normalized_name FROM ingredient_catalog WHERE id = ?")
                .bind(&canonical_id)
                .fetch_one(&mut **transaction)
                .await?;
        if canonical_normalized != normalized {
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
               AND (r.owner_id = ? OR r.visibility = 'public')
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

async fn resolve_recommendation_request(
    pool: &SqlitePool,
    owner_id: &str,
    request: &mut RecommendationRequest,
) -> AppResult<()> {
    for references in [
        &mut request.preferred,
        &mut request.required,
        &mut request.excluded,
    ] {
        for reference in references.iter_mut() {
            if let crate::models::IngredientReference::Selection {
                name,
                canonical_ingredient_id,
            } = reference
            {
                if canonical_ingredient_id.is_none() {
                    *canonical_ingredient_id = resolve_reference_id(pool, owner_id, name).await?;
                } else if !canonical_ingredient_is_visible_pool(
                    pool,
                    owner_id,
                    canonical_ingredient_id.as_deref().unwrap_or_default(),
                )
                .await?
                {
                    return Err(AppError::Validation(
                        "selected canonical ingredient is not available to this account".into(),
                    ));
                }
            } else {
                let name = reference.name().to_string();
                if let Some(canonical_ingredient_id) =
                    resolve_reference_id(pool, owner_id, &name).await?
                {
                    *reference = crate::models::IngredientReference::Selection {
                        name,
                        canonical_ingredient_id: Some(canonical_ingredient_id),
                    };
                }
            }
        }
    }
    Ok(())
}

async fn resolve_reference_id(
    pool: &SqlitePool,
    owner_id: &str,
    name: &str,
) -> AppResult<Option<String>> {
    let normalized = normalize_ingredient(name);
    if normalized.is_empty() {
        return Ok(None);
    }
    let alias: Option<String> = sqlx::query_scalar(
        "SELECT canonical_ingredient_id FROM ingredient_aliases
         WHERE owner_id = ? AND alias_normalized = ?",
    )
    .bind(owner_id)
    .bind(&normalized)
    .fetch_optional(pool)
    .await?;
    if alias.is_some() {
        return Ok(alias);
    }
    Ok(
        sqlx::query_scalar("SELECT id FROM ingredient_catalog WHERE normalized_name = ?")
            .bind(normalized)
            .fetch_optional(pool)
            .await?,
    )
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
               AND (r.owner_id = ? OR r.visibility = 'public')
           )
           OR EXISTS (SELECT 1 FROM ingredient_aliases a
                      WHERE a.canonical_ingredient_id = c.id AND a.owner_id = ?)
         )",
    )
    .bind(canonical_id)
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
    if match_reason(query, canonical, &[]) == Some(reason) {
        return canonical.to_string();
    }
    aliases
        .iter()
        .find(|alias| {
            let alias_reason = match match_reason(query, alias, &[]) {
                Some(MatchReason::Exact) => Some(MatchReason::Alias),
                value => value,
            };
            alias_reason == Some(reason)
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
        "INSERT INTO recipes (id, owner_id, title, description, instructions, visibility) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(owner_id)
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(input.instructions.trim())
    .bind(&input.visibility)
    .execute(&mut *transaction)
    .await?;
    insert_ingredients(&mut transaction, id, owner_id, &input.ingredients).await?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn replace_ingredients(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recipe_id: &str,
    owner_id: &str,
    ingredients: &[IngredientInput],
) -> AppResult<()> {
    let ingredients = resolve_ingredients(transaction, owner_id, ingredients).await?;
    sqlx::query("DELETE FROM recipe_ingredients WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut **transaction)
        .await?;
    insert_resolved_ingredients(transaction, recipe_id, &ingredients).await
}

pub(crate) async fn insert_ingredients(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recipe_id: &str,
    owner_id: &str,
    ingredients: &[IngredientInput],
) -> AppResult<()> {
    let ingredients = resolve_ingredients(transaction, owner_id, ingredients).await?;
    insert_resolved_ingredients(transaction, recipe_id, &ingredients).await
}

async fn resolve_ingredients(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_id: &str,
    ingredients: &[IngredientInput],
) -> AppResult<Vec<(IngredientInput, String)>> {
    let ingredients = expand_ingredient_inputs(ingredients.to_vec());
    let mut resolved = Vec::with_capacity(ingredients.len());
    for ingredient in ingredients {
        let canonical_id = resolve_canonical_ingredient(
            transaction,
            owner_id,
            &ingredient.name,
            ingredient.canonical_ingredient_id.as_deref(),
        )
        .await?;
        resolved.push((ingredient, canonical_id));
    }
    Ok(resolved)
}

async fn insert_resolved_ingredients(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recipe_id: &str,
    ingredients: &[(IngredientInput, String)],
) -> AppResult<()> {
    for (index, (ingredient, canonical_id)) in ingredients.iter().enumerate() {
        sqlx::query(
            "INSERT INTO recipe_ingredients
             (id, recipe_id, name, normalized_name, quantity_note, sort_order,
              canonical_ingredient_id)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(recipe_id)
        .bind(clean_display_name(&ingredient.name))
        .bind(normalize_ingredient(&ingredient.name))
        .bind(ingredient.quantity_note.trim())
        .bind(
            i64::try_from(index)
                .map_err(|_| AppError::Validation("too many ingredients".into()))?,
        )
        .bind(canonical_id)
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
            "SELECT r.id, r.owner_id, r.title, r.description, r.instructions, r.visibility, r.version,
               u.public_nickname, r.source_url, r.source_title, r.source_author,
               r.source_warnings, r.source_confidence,
               EXISTS(SELECT 1 FROM favorites f WHERE f.user_id = ? AND f.recipe_id = r.id) AS is_favorite
             FROM recipes r JOIN users u ON u.id = r.owner_id
             WHERE r.id = ? AND (r.owner_id = ? OR r.visibility = 'public')",
        )
        .bind(user_id)
        .bind(id)
        .bind(user_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT r.id, r.owner_id, r.title, r.description, r.instructions, r.visibility, r.version,
               u.public_nickname, r.source_url, r.source_title, r.source_author,
               r.source_warnings, r.source_confidence,
               EXISTS(SELECT 1 FROM favorites f WHERE f.user_id = ? AND f.recipe_id = r.id) AS is_favorite
             FROM recipes r JOIN users u ON u.id = r.owner_id
             WHERE r.owner_id = ? OR r.visibility = 'public' ORDER BY r.updated_at DESC",
        )
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
        let source = if let Some(url) = source_url {
            let warnings = match row.get::<Option<String>, _>("source_warnings") {
                Some(value) => serde_json::from_str(&value).map_err(|error| {
                    AppError::Data(format!("invalid stored recipe source warnings: {error}"))
                })?,
                None => Vec::new(),
            };
            Some(RecipeSource {
                url,
                title: row.get("source_title"),
                author: row.get("source_author"),
                warnings,
                confidence: row.get("source_confidence"),
            })
        } else {
            None
        };
        recipes.push(Recipe {
            ingredients: load_ingredients(pool, &recipe_id).await?,
            cover_image: load_recipe_image_meta(pool, &recipe_id).await?,
            id: recipe_id,
            is_owner: owner_id == user_id,
            owner_id,
            author_nickname: row.get("public_nickname"),
            title: row.get("title"),
            description: row.get("description"),
            instructions: row.get("instructions"),
            visibility: row.get("visibility"),
            version: row.get("version"),
            is_favorite: row.get::<i64, _>("is_favorite") != 0,
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
    pool: &SqlitePool,
    user_id: &str,
    settings: &RecommendationSettings,
) -> AppResult<Vec<Recipe>> {
    let all = load_visible_recipes(pool, user_id, None).await?;
    Ok(all
        .into_iter()
        .filter(|recipe| {
            (settings.include_own && recipe.owner_id == user_id)
                || (settings.include_public
                    && recipe.owner_id != user_id
                    && recipe.visibility == "public")
                || (settings.include_favorites && recipe.is_favorite)
        })
        .collect())
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

async fn ensure_visible_recipe(pool: &SqlitePool, user_id: &str, recipe_id: &str) -> AppResult<()> {
    let exists: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM recipes WHERE id = ? AND (owner_id = ? OR visibility = 'public')",
    )
    .bind(recipe_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    exists.map(|_| ()).ok_or(AppError::NotFound)
}

async fn load_pantry(pool: &SqlitePool, user_id: &str) -> AppResult<Vec<PantryItem>> {
    type PantryRow = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
    );
    let rows: Vec<PantryRow> = sqlx::query_as(
        "SELECT id, name, status, quantity_note, confirmed_at, expires_on,
                canonical_ingredient_id, auto_add_to_shopping_list_when_out
         FROM pantry_items WHERE owner_id = ? ORDER BY normalized_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
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
            )| PantryItem {
                id,
                name,
                status: PantryStatus::parse(&status),
                quantity_note,
                confirmed_at,
                expires_on,
                canonical_ingredient_id,
                auto_add_to_shopping_list_when_out,
            },
        )
        .collect())
}

async fn load_settings(pool: &SqlitePool, user_id: &str) -> AppResult<RecommendationSettings> {
    let row: Option<(bool, bool, bool, i64)> = sqlx::query_as(
        "SELECT include_favorites, include_own, include_public, verification_days
         FROM recommendation_settings WHERE owner_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row
        .map(
            |(include_favorites, include_own, include_public, verification_days)| {
                RecommendationSettings {
                    include_favorites,
                    include_own,
                    include_public,
                    verification_days,
                }
            },
        )
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc};

    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use chrono::Utc;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use serde_json::{json, Value};
    use sqlx::{Row, SqlitePool};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        auth::{build_auth, AuthRuntime},
        build_app,
        config::{AppEnvironment, AuthMode, Config},
        db,
        passwordless::GOOGLE_IDENTITY_ISSUER,
        AppState,
    };

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
            "title": "Imported soup",
            "description": "Review me",
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
        let boundary = "cook-test-boundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"altText\"\r\n\r\nBlue soup\r\n"
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
        assert_eq!(metadata_update.status(), StatusCode::NO_CONTENT);
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
            "public, max-age=31536000, immutable"
        );
        let make_private = app
            .clone()
            .oneshot(
                Request::put(format!("/api/recipes/{recipe_id}"))
                    .header("content-type", "application/json")
                    .header("cookie", &owner_cookie)
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
            .oneshot(Request::get(&image_url).body(Body::empty()).unwrap())
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
        assert_eq!(recipe["source"]["warnings"][0], "Timing was inferred.");

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
    async fn automatic_shopping_items_follow_out_pantry_state_and_purchase_atomically() {
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
                Request::post(format!("/api/shopping-list/automatic/{milk_id}/purchased"))
                    .header("cookie", &other_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cross_account.status(), StatusCode::NOT_FOUND);

        let purchased = app
            .clone()
            .oneshot(
                Request::post(format!("/api/shopping-list/automatic/{milk_id}/purchased"))
                    .header("cookie", &owner_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(purchased.status(), StatusCode::OK);
        let purchased = response_json(purchased).await;
        assert_eq!(purchased.as_array().unwrap().len(), 1);
        assert_eq!(purchased[0]["automatic"], false);
        assert!(purchased[0]["completedAt"].is_string());
        let pantry = app
            .oneshot(
                Request::get("/api/pantry")
                    .header("cookie", owner_cookie)
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
            .oneshot(
                Request::get("/api/shopping-list")
                    .header("cookie", cookie)
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
                Request::get("/api/ingredients/suggestions?q=tomtao")
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
        let powder = body
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["title"] == "Powder test")
            .unwrap();
        assert_eq!(powder["status"], "BLOCKED");
        assert_eq!(powder["missing"], json!(["Baking powder"]));
        let tomato = body
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["recipe"]["title"] == "Tomato plate")
            .unwrap();
        assert_eq!(tomato["status"], "READY");
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

use axum::{
    extract::{Path, Query, State},
    http::{header::SET_COOKIE, HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
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
    auth::{load_user, upsert_development_user},
    config::AuthMode,
    error::{AppError, AppResult},
    imports,
    models::{
        normalize_ingredient, normalize_nickname, IngredientInput, PantryItem, PantryItemInput,
        PantryStatus, ProfileInput, Recipe, RecipeInput, RecipeSource, RecommendationRequest,
        RecommendationSettings, User,
    },
    passwordless::{
        email_address, map_email_error, map_passkey_error, normalize_email, purpose,
        PasskeySummary, EMAIL_LINK_PURPOSE, EMAIL_SIGN_IN_PURPOSE,
    },
    recommendation::{recommend, Recommendation},
    AppState,
};

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/me", get(me))
        .route("/api/settings/profile", put(update_profile))
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
        .route("/api/pantry", get(list_pantry).put(upsert_pantry))
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
    passkeys: Vec<PasskeySummary>,
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
    let challenge = request_email_otp(
        &state,
        &headers,
        &input.email,
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
    let login = state.auth.login.as_ref().ok_or(AppError::NotFound)?;
    let completion = login
        .complete(
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
        .map_err(map_login_error)?;
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
    Ok(SecuritySettings {
        linked_emails: state.auth.passwordless.store.linked_emails(user_id).await?,
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
    Json(input): Json<RecipeInput>,
) -> AppResult<(StatusCode, Json<Recipe>)> {
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
    Json(input): Json<RecipeInput>,
) -> AppResult<Json<Recipe>> {
    validate_recipe(&input)?;
    let mut transaction = state.pool.begin().await?;
    let result = sqlx::query(
        "UPDATE recipes SET title = ?, description = ?, instructions = ?, visibility = ?, updated_at = CURRENT_TIMESTAMP
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
    replace_ingredients(&mut transaction, &id, &input.ingredients).await?;
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
        sqlx::query(
            "INSERT INTO pantry_items (id, owner_id, name, normalized_name, status, quantity_note, confirmed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(owner_id, normalized_name) DO UPDATE SET
               name = excluded.name, status = excluded.status, quantity_note = excluded.quantity_note,
               confirmed_at = excluded.confirmed_at, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&user.id)
        .bind(item.name.trim())
        .bind(normalized)
        .bind(item.status.as_str())
        .bind(item.quantity_note.trim())
        .bind(Utc::now().to_rfc3339())
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
    Json(request): Json<RecommendationRequest>,
) -> AppResult<Json<Vec<Recommendation>>> {
    let settings = load_settings(&state.pool, &user.id).await?;
    let candidates = load_candidate_recipes(&state.pool, &user.id, &settings).await?;
    let pantry = load_pantry(&state.pool, &user.id).await?;
    Ok(Json(recommend(
        candidates,
        &pantry,
        &request,
        settings.verification_days,
        Utc::now(),
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

fn validate_recipe(input: &RecipeInput) -> AppResult<()> {
    if input.title.trim().is_empty() {
        return Err(AppError::Validation("title is required".into()));
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
    if input.visibility != "private" && input.visibility != "public" {
        return Err(AppError::Validation(
            "visibility must be private or public".into(),
        ));
    }
    Ok(())
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
    insert_ingredients(&mut transaction, id, &input.ingredients).await?;
    transaction.commit().await?;
    Ok(())
}

async fn replace_ingredients(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recipe_id: &str,
    ingredients: &[IngredientInput],
) -> AppResult<()> {
    sqlx::query("DELETE FROM recipe_ingredients WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut **transaction)
        .await?;
    insert_ingredients(transaction, recipe_id, ingredients).await
}

pub(crate) async fn insert_ingredients(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    recipe_id: &str,
    ingredients: &[IngredientInput],
) -> AppResult<()> {
    for (index, ingredient) in ingredients.iter().enumerate() {
        sqlx::query(
            "INSERT INTO recipe_ingredients (id, recipe_id, name, normalized_name, quantity_note, sort_order)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(recipe_id)
        .bind(ingredient.name.trim())
        .bind(normalize_ingredient(&ingredient.name))
        .bind(ingredient.quantity_note.trim())
        .bind(index as i64)
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
            "SELECT r.id, r.owner_id, r.title, r.description, r.instructions, r.visibility,
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
            id: recipe_id,
            is_owner: owner_id == user_id,
            owner_id,
            author_nickname: row.get("public_nickname"),
            title: row.get("title"),
            description: row.get("description"),
            instructions: row.get("instructions"),
            visibility: row.get("visibility"),
            is_favorite: row.get::<i64, _>("is_favorite") != 0,
            source,
        });
    }
    Ok(recipes)
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

async fn load_ingredients(pool: &SqlitePool, recipe_id: &str) -> AppResult<Vec<IngredientInput>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, quantity_note FROM recipe_ingredients WHERE recipe_id = ? ORDER BY sort_order",
    )
    .bind(recipe_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, quantity_note)| IngredientInput {
            name,
            quantity_note,
        })
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
    let rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, name, status, quantity_note, confirmed_at
         FROM pantry_items WHERE owner_id = ? ORDER BY normalized_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, status, quantity_note, confirmed_at)| PantryItem {
                id,
                name,
                status: PantryStatus::parse(&status),
                quantity_note,
                confirmed_at,
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
    use std::sync::Arc;

    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use chrono::Utc;
    use serde_json::{json, Value};
    use sqlx::SqlitePool;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        auth::{build_auth, AuthRuntime},
        build_app,
        config::{AppEnvironment, AuthMode, Config},
        db, AppState,
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

    async fn create_import_token(app: &axum::Router, cookie: &str) -> Value {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/settings/import-tokens")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(Body::from(
                        json!({"name": "Test client", "expiresInDays": 90}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        response_json(response).await
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

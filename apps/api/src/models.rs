use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub onboarding_completed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileInput {
    pub nickname: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngredientInput {
    pub name: String,
    #[serde(default)]
    pub quantity_note: String,
    #[serde(default)]
    pub canonical_ingredient_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default = "private_visibility")]
    pub visibility: String,
    pub ingredients: Vec<IngredientInput>,
}

fn private_visibility() -> String {
    "private".into()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: String,
    #[serde(skip_serializing)]
    pub owner_id: String,
    pub is_owner: bool,
    pub author_nickname: Option<String>,
    pub title: String,
    pub description: String,
    pub instructions: String,
    pub visibility: String,
    pub ingredients: Vec<IngredientInput>,
    pub is_favorite: bool,
    pub source: Option<RecipeSource>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSource {
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub warnings: Vec<String>,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PantryItemInput {
    pub name: String,
    pub status: PantryStatus,
    #[serde(default)]
    pub quantity_note: String,
    #[serde(default, deserialize_with = "deserialize_nullable_update")]
    pub expires_on: NullableUpdate<String>,
    #[serde(default)]
    pub canonical_ingredient_id: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub enum NullableUpdate<T> {
    #[default]
    Missing,
    Value(Option<T>),
}

fn deserialize_nullable_update<'de, D, T>(deserializer: D) -> Result<NullableUpdate<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(NullableUpdate::Value)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PantryItem {
    pub id: String,
    pub name: String,
    pub status: PantryStatus,
    pub quantity_note: String,
    pub confirmed_at: Option<String>,
    pub expires_on: Option<String>,
    pub canonical_ingredient_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PantryStatus {
    Have,
    Out,
    Unknown,
}

impl PantryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Have => "have",
            Self::Out => "out",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "have" => Self::Have,
            "out" => Self::Out,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationSettings {
    pub include_favorites: bool,
    pub include_own: bool,
    pub include_public: bool,
    pub verification_days: i64,
}

impl Default for RecommendationSettings {
    fn default() -> Self {
        Self {
            include_favorites: true,
            include_own: true,
            include_public: true,
            verification_days: 14,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationRequest {
    #[serde(default)]
    pub preferred: Vec<IngredientReference>,
    #[serde(default)]
    pub required: Vec<IngredientReference>,
    #[serde(default)]
    pub excluded: Vec<IngredientReference>,
    #[serde(default)]
    pub pantry_only: bool,
    #[serde(default)]
    pub today: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum IngredientReference {
    Name(String),
    Selection {
        name: String,
        #[serde(rename = "canonicalIngredientId")]
        canonical_ingredient_id: Option<String>,
    },
}

impl IngredientReference {
    pub fn name(&self) -> &str {
        match self {
            Self::Name(name) | Self::Selection { name, .. } => name,
        }
    }

    pub fn canonical_ingredient_id(&self) -> Option<&str> {
        match self {
            Self::Name(_) => None,
            Self::Selection {
                canonical_ingredient_id,
                ..
            } => canonical_ingredient_id.as_deref(),
        }
    }
}

impl From<&str> for IngredientReference {
    fn from(value: &str) -> Self {
        Self::Name(value.into())
    }
}

pub fn normalize_nickname(value: Option<&str>) -> Result<Option<String>, &'static str> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > 40 {
        return Err("nickname must be at most 40 characters");
    }
    let mut previous_space = false;
    for character in value.chars() {
        if character.is_control() {
            return Err("nickname cannot contain control characters");
        }
        if character.is_whitespace() && character != ' ' {
            return Err("nickname can only use regular spaces as whitespace");
        }
        if character == ' ' {
            if previous_space {
                return Err("nickname cannot contain repeated spaces");
            }
            previous_space = true;
        } else {
            previous_space = false;
        }
    }
    Ok(Some(value.to_string()))
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngredientInput {
    pub name: String,
    #[serde(default)]
    pub quantity_note: String,
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
    pub owner_id: String,
    pub title: String,
    pub description: String,
    pub instructions: String,
    pub visibility: String,
    pub ingredients: Vec<IngredientInput>,
    pub is_favorite: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PantryItemInput {
    pub name: String,
    pub status: PantryStatus,
    #[serde(default)]
    pub quantity_note: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PantryItem {
    pub id: String,
    pub name: String,
    pub status: PantryStatus,
    pub quantity_note: String,
    pub confirmed_at: Option<String>,
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
    pub preferred: Vec<String>,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub excluded: Vec<String>,
    #[serde(default)]
    pub pantry_only: bool,
}

pub fn normalize_ingredient(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

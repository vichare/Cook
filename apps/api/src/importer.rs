use async_trait::async_trait;

use crate::{
    error::AppResult,
    models::{IngredientInput, RecipeInput},
};

pub struct LicensedRecipe {
    pub recipe: RecipeInput,
    pub source_url: String,
    pub license: String,
    pub attribution: String,
}

#[async_trait]
pub trait LicensedRecipeImporter: Send + Sync {
    async fn import(&self) -> AppResult<Vec<LicensedRecipe>>;
}

pub struct FixtureImporter;

#[async_trait]
impl LicensedRecipeImporter for FixtureImporter {
    async fn import(&self) -> AppResult<Vec<LicensedRecipe>> {
        Ok(vec![LicensedRecipe {
            recipe: RecipeInput {
                title: "Licensed fixture toast".into(),
                description: "Synthetic fixture content for importer tests.".into(),
                instructions: "Toast the bread.".into(),
                visibility: "private".into(),
                difficulty: None,
                estimated_calories: None,
                ingredients: vec![IngredientInput {
                    name: "bread".into(),
                    quantity_note: "one slice".into(),
                    canonical_ingredient_id: None,
                }],
            },
            source_url: "https://example.test/recipes/licensed-fixture".into(),
            license: "CC0-1.0".into(),
            attribution: "Cook test fixture".into(),
        }])
    }
}

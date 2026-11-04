# Public recipe query inventory

Every non-owner recipe authorization decision must resolve through `public_eligible_recipes`, whose invariant is:

`publication_intent = 'public' AND moderation_state = 'clear' AND visibility = 'public'`

The database triggers in `0017_content_moderation.sql` keep the legacy effective `visibility` field consistent and prevent old browser or AI clients from publishing a restricted recipe.

| Surface | Implementation | Required behavior for restricted recipes |
|---|---|---|
| Recipe detail and general recipe list | `routes.rs` recipe hydration | Other users receive 404/exclusion; owner still receives the recipe |
| Public author profile and pagination | `routes.rs` author queries | Excluded |
| Favorite recipes and favorite accounts | `routes.rs` candidate/source queries | Removed dynamically |
| Recommendation selection, hydration, and final recheck | `routes.rs` recommendation queries | Excluded at selection and discarded if restricted during a race |
| Ingredient suggestions and canonical visibility | `routes.rs` ingredient queries | Ingredients from restricted recipes do not make catalog identities visible |
| Add recipe ingredient to shopping list | `shopping.rs` | Other users receive 404 |
| Public cover image bytes and conditional requests | `images.rs` | Old random image URL immediately returns 404; reinstate exposes a new ID |
| AI recipe reads | `ai_recipes.rs` | Tokens remain owner-scoped; no cross-account public authorization is added |
| Moderation reporting | `moderation.rs` | Only current public-clear non-owned recipes are reportable |
| Moderation queue | `moderation.rs` | Includes public unreviewed content and active cases/appeals, never ordinary private drafts |

Release review must search for new `FROM recipes`, `JOIN recipes`, image, author, favorite, search, recommendation, and shopping queries. Any new non-owner surface must use the shared view or an equivalent ID membership check and add a restriction test.

//! Explicit manual model list.
//!
//! `/v1/models` is recorded as a catalog observation. It does not add a
//! selectable row and it does not mark anything verified. Claude ids are
//! rejected. There is no fallback from one model to another.

use crate::pin::sha256_bytes;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    ChatGptCodex,
    Antigravity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    Discovered {
        observed_at: String,
        validation_source: String,
    },
    Verified {
        verified_at: String,
        validation_source: String,
        live: bool,
    },
    Unavailable {
        observed_at: String,
        validation_source: String,
        class: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyEvidence {
    /// Catalog or credential listing. Never accepted as verification.
    Discovery,
    MockFixture,
    UserPresentTurn,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelEntry {
    pub model_id: String,
    pub alias: Option<String>,
    pub source: SourceKind,
    pub account_ref: String,
    pub status: ModelStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogObservation {
    pub seen: usize,
    pub rejected_claude: usize,
    pub unclassified: usize,
    pub rejected_ids: Vec<String>,
    pub selectable_added: usize,
}

#[derive(Debug, Default)]
pub struct ModelRegistry {
    entries: Vec<ModelEntry>,
    pub user_present_verifications: u32,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe_gateway_catalog(&self, models: &[CatalogModel]) -> CatalogObservation {
        let mut rejected_ids = Vec::new();
        let mut unclassified = 0;
        for model in models {
            if is_claude(&model.id, model.owned_by.as_deref()) {
                rejected_ids.push(model.id.clone());
            } else {
                unclassified += 1;
            }
        }
        CatalogObservation {
            seen: models.len(),
            rejected_claude: rejected_ids.len(),
            unclassified,
            rejected_ids,
            selectable_added: 0,
        }
    }

    pub fn insert_explicit(
        &mut self,
        alias: Option<&str>,
        model_id: &str,
        account_ref: &str,
        source: SourceKind,
    ) -> Result<(), String> {
        if is_claude(model_id, None) {
            return Err("claude is outside the manual list".into());
        }
        if model_id.is_empty() || account_ref.is_empty() {
            return Err("model id and account ref are required".into());
        }
        self.entries.push(ModelEntry {
            model_id: model_id.to_string(),
            alias: alias.map(str::to_string),
            source,
            account_ref: public_account(account_ref),
            status: ModelStatus::Discovered {
                observed_at: "unspecified".into(),
                validation_source: "explicit-manual".into(),
            },
        });
        Ok(())
    }

    pub fn ingest_credential_models(
        &mut self,
        provider: &str,
        account_ref: &str,
        model_ids: &[String],
    ) -> Result<usize, String> {
        let source = match provider {
            "codex" => SourceKind::ChatGptCodex,
            "antigravity" => SourceKind::Antigravity,
            other => {
                return Err(format!(
                    "provider {other} is not part of the manual list; nothing was added"
                ))
            }
        };
        let mut added = 0;
        for id in model_ids {
            if is_claude(id, None) {
                continue;
            }
            self.insert_explicit(None, id, account_ref, source)?;
            if let Some(entry) = self.entries.last_mut() {
                entry.status = ModelStatus::Discovered {
                    observed_at: "unspecified".into(),
                    validation_source: "credential-models".into(),
                };
            }
            added += 1;
        }
        Ok(added)
    }

    pub fn note_verified(&mut self, model_id: &str, evidence: VerifyEvidence) -> Result<(), String> {
        match evidence {
            VerifyEvidence::Discovery => {
                Err("a discovery response cannot mark a model verified".into())
            }
            VerifyEvidence::MockFixture => {
                self.set_verified(model_id, "mock-fixture", false)
            }
            VerifyEvidence::UserPresentTurn => {
                self.user_present_verifications += 1;
                self.set_verified(model_id, "user-present-turn", true)
            }
        }
    }

    pub fn note_unavailable(&mut self, model_id: &str, class: &str) -> Result<(), String> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.model_id == model_id)
            .ok_or_else(|| format!("unknown model {model_id}"))?;
        entry.status = ModelStatus::Unavailable {
            observed_at: "unspecified".into(),
            validation_source: "explicit-status".into(),
            class: class.to_string(),
        };
        Ok(())
    }

    pub fn resolve(&self, name: &str) -> Result<&ModelEntry, String> {
        self.entries
            .iter()
            .find(|e| e.model_id == name || e.alias.as_deref() == Some(name))
            .ok_or_else(|| format!("no exact model or alias {name}; fallback is disabled"))
    }

    pub fn selectable(&self) -> Vec<&ModelEntry> {
        self.entries.iter().collect()
    }

    pub fn export_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.entries).unwrap_or(serde_json::Value::Null)
    }

    fn set_verified(&mut self, model_id: &str, source: &str, live: bool) -> Result<(), String> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.model_id == model_id)
            .ok_or_else(|| format!("unknown model {model_id}"))?;
        entry.status = ModelStatus::Verified {
            verified_at: "unspecified".into(),
            validation_source: source.into(),
            live,
        };
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CatalogModel {
    pub id: String,
    pub owned_by: Option<String>,
}

pub fn is_claude(id: &str, owned_by: Option<&str>) -> bool {
    let id = id.to_ascii_lowercase();
    let owned = owned_by.unwrap_or("").to_ascii_lowercase();
    id.contains("claude") || owned.contains("anthropic") || owned == "claude"
}

fn public_account(account_ref: &str) -> String {
    if account_ref.contains('@') || account_ref.to_ascii_lowercase().contains("token") {
        format!("sha256:{}", &sha256_bytes(account_ref.as_bytes())[..12])
    } else {
        account_ref.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_does_not_select_or_verify_and_drops_claude() {
        let mut registry = ModelRegistry::new();
        let obs = registry.observe_gateway_catalog(&[
            CatalogModel {
                id: "claude-opus".into(),
                owned_by: Some("anthropic".into()),
            },
            CatalogModel {
                id: "gpt-example".into(),
                owned_by: Some("openai".into()),
            },
            CatalogModel {
                id: "gemini-example".into(),
                owned_by: Some("google".into()),
            },
        ]);
        assert_eq!(obs.rejected_claude, 1);
        assert_eq!(obs.selectable_added, 0);
        assert!(registry.selectable().is_empty());
        assert!(registry
            .note_verified("gpt-example", VerifyEvidence::Discovery)
            .is_err());
        registry
            .insert_explicit(
                Some("sol"),
                "gpt-example",
                "acct-1",
                SourceKind::ChatGptCodex,
            )
            .unwrap();
        assert!(registry.resolve("other-model").is_err());
        assert_eq!(registry.resolve("sol").unwrap().model_id, "gpt-example");
        registry
            .note_verified("gpt-example", VerifyEvidence::MockFixture)
            .unwrap();
        match &registry.resolve("sol").unwrap().status {
            ModelStatus::Verified { live, .. } => assert!(!live),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(registry.user_present_verifications, 0);
    }

    #[test]
    fn credential_ingest_keeps_source_and_hides_email() {
        let mut registry = ModelRegistry::new();
        assert!(registry
            .ingest_credential_models("claude", "acct", &["claude-opus".into()])
            .is_err());
        assert!(registry.selectable().is_empty());
        let added = registry
            .ingest_credential_models(
                "antigravity",
                "person@example.com",
                &["gemini-example".into(), "claude-hidden".into()],
            )
            .unwrap();
        assert_eq!(added, 1);
        let exported = registry.export_json().to_string();
        assert!(!exported.contains("person@example.com"));
        assert!(!exported.contains("claude-hidden"));
        assert_eq!(
            registry.selectable()[0].source,
            SourceKind::Antigravity
        );
    }
}

use bb_wire::prompt::{
    ExecutionInputSource, ExistingThreadExecutionInputSources, PermissionMode, ReasoningLevel,
    ResolvedThreadExecutionOptions,
};
use bb_wire::system::{AvailableModel, ProviderInfo, SystemExecutionOptionsResponse};
use bb_wire::timeline::ModelFallback;

use crate::prompt::{
    DefaultsState, ExecutionSelection, PERMISSION_MODE_OPTIONS, permission_mode_rank,
    reasoning_fallback_label, reconcile_reasoning_level, resolve_default_execution_options_state,
    resolve_permission_mode_selection,
};

const DEFAULT_SERVICE_TIER: &str = "default";

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Loaded(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Self::Loaded(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    pub fn from_result(result: Result<T, String>) -> Self {
        match result {
            Ok(value) => Self::Loaded(value),
            Err(error) => Self::Failed(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerOption<T> {
    pub value: T,
    pub label: String,
    pub detail: Option<String>,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExecutionState {
    pub provider_id: String,
    pub options: Load<SystemExecutionOptionsResponse>,
    pub defaults: Load<Option<ResolvedThreadExecutionOptions>>,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub permission: Option<PermissionMode>,
    pub service_tier: Option<String>,
    pub overridden_fallback: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedExecution {
    pub model: String,
    pub model_label: String,
    pub active_model: Option<AvailableModel>,
    pub reasoning: ReasoningLevel,
    pub reasoning_options: Vec<PickerOption<ReasoningLevel>>,
    pub permission: PermissionMode,
    pub permission_options: Vec<PickerOption<PermissionMode>>,
    pub model_options: Vec<PickerOption<String>>,
    pub service_tier: Option<String>,
    pub supports_service_tier: bool,
    pub fallback_active: bool,
    pub unavailable_model_recovery: bool,
    pub sources: ExistingThreadExecutionInputSources,
    pub defaults_state: DefaultsState,
}

pub fn format_model_label(value: &str) -> String {
    value
        .split('-')
        .map(|part| {
            if part.eq_ignore_ascii_case("gpt") {
                "GPT".to_owned()
            } else if !part.is_empty()
                && part.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
                && !part.starts_with('.')
                && !part.ends_with('.')
            {
                part.to_owned()
            } else if !part.is_empty() && part.chars().all(|ch| ch.is_ascii_alphabetic()) {
                let mut chars = part.chars();
                let first = chars.next().map(|ch| ch.to_ascii_uppercase());
                first
                    .into_iter()
                    .chain(chars.map(|ch| ch.to_ascii_lowercase()))
                    .collect()
            } else {
                part.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

pub fn reasoning_label(level: ReasoningLevel, provider: Option<&ProviderInfo>) -> String {
    provider
        .and_then(|provider| provider.reasoning_levels.as_ref())
        .and_then(|levels| levels.iter().find(|option| option.id == level.as_str()))
        .map(|option| option.label.clone())
        .unwrap_or_else(|| reasoning_fallback_label(level).to_owned())
}

fn model_option(model: &AvailableModel) -> PickerOption<String> {
    let name = if model.display_name.is_empty() {
        &model.model
    } else {
        &model.display_name
    };
    PickerOption {
        value: model.model.clone(),
        label: format_model_label(name),
        detail: (!model.description.is_empty()).then(|| model.description.clone()),
        disabled: false,
    }
}

pub struct CatalogSelection {
    pub selected_model: String,
    pub active_model: Option<AvailableModel>,
    pub model_options: Vec<PickerOption<String>>,
    pub reasoning: ReasoningLevel,
    pub reasoning_options: Vec<PickerOption<ReasoningLevel>>,
    pub unavailable_model_recovery: bool,
}

pub fn resolve_model_catalog_selection(
    models: &[AvailableModel],
    selected_only: &[AvailableModel],
    raw_model: &str,
    preferred_reasoning: ReasoningLevel,
    provider: Option<&ProviderInfo>,
    catalog_verified: bool,
) -> CatalogSelection {
    let full: Vec<&AvailableModel> = models.iter().chain(selected_only).collect();
    let selection = if raw_model.is_empty() || full.iter().any(|model| model.model == raw_model) {
        raw_model.to_owned()
    } else {
        let suffix = format!("/{raw_model}");
        let prefixed: Vec<&&AvailableModel> = full
            .iter()
            .filter(|model| model.model.ends_with(&suffix))
            .collect();
        match prefixed.as_slice() {
            [single] => single.model.clone(),
            _ => raw_model.to_owned(),
        }
    };
    let mut available: Vec<AvailableModel> = models.to_vec();
    if !selection.is_empty()
        && !available.iter().any(|model| model.model == selection)
        && let Some(extra) = selected_only.iter().find(|model| model.model == selection)
    {
        available.insert(0, extra.clone());
    }
    let selected_model = if (!catalog_verified && !selection.is_empty())
        || available.is_empty()
        || available.iter().any(|model| model.model == selection)
    {
        selection
    } else {
        available
            .iter()
            .find(|model| model.is_default)
            .unwrap_or(&available[0])
            .model
            .clone()
    };
    let active_model = available
        .iter()
        .find(|model| model.model == selected_model)
        .or_else(|| available.iter().find(|model| model.is_default))
        .or_else(|| available.first())
        .cloned();
    let mut reasoning_options: Vec<PickerOption<ReasoningLevel>> = Vec::new();
    for effort in active_model
        .iter()
        .flat_map(|model| model.supported_reasoning_efforts.iter())
    {
        if reasoning_options
            .iter()
            .any(|option| option.value == effort.reasoning_effort)
        {
            continue;
        }
        reasoning_options.push(PickerOption {
            value: effort.reasoning_effort,
            label: reasoning_label(effort.reasoning_effort, provider),
            detail: (!effort.description.is_empty()).then(|| effort.description.clone()),
            disabled: false,
        });
    }
    let supported: Vec<ReasoningLevel> = reasoning_options
        .iter()
        .map(|option| option.value)
        .collect();
    let reasoning =
        reconcile_reasoning_level(preferred_reasoning, &supported).unwrap_or(preferred_reasoning);
    let mut model_options: Vec<PickerOption<String>> = available.iter().map(model_option).collect();
    model_options.extend(
        selected_only
            .iter()
            .filter(|model| !available.iter().any(|active| active.model == model.model))
            .map(model_option),
    );
    CatalogSelection {
        unavailable_model_recovery: catalog_verified
            && !raw_model.is_empty()
            && selected_model != raw_model,
        selected_model,
        active_model,
        model_options,
        reasoning,
        reasoning_options,
    }
}

fn service_tier_options(
    provider: Option<&ProviderInfo>,
    model: Option<&AvailableModel>,
) -> Vec<String> {
    let declared: Vec<String> = provider
        .and_then(|provider| provider.service_tiers.as_ref())
        .map(|tiers| tiers.iter().map(|tier| tier.id.clone()).collect())
        .unwrap_or_default();
    match model.and_then(|model| model.supported_service_tiers.as_ref()) {
        None => declared,
        Some(supported) => declared
            .into_iter()
            .filter(|id| supported.iter().any(|tier| &tier.id == id))
            .collect(),
    }
}

impl ExecutionState {
    pub fn new(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            ..Self::default()
        }
    }

    pub fn defaults(&self) -> Option<&ResolvedThreadExecutionOptions> {
        self.defaults.loaded().and_then(Option::as_ref)
    }

    pub fn defaults_state(&self) -> DefaultsState {
        resolve_default_execution_options_state(
            self.defaults().is_some(),
            self.defaults.loaded().is_some(),
            self.defaults.is_failed(),
        )
    }

    pub fn provider(&self) -> Option<&ProviderInfo> {
        self.options
            .loaded()?
            .providers
            .iter()
            .find(|provider| provider.id == self.provider_id)
    }

    pub fn touched(&self) -> bool {
        self.model.is_some()
            || self.reasoning.is_some()
            || self.permission.is_some()
            || self.service_tier.is_some()
    }

    pub fn fallback_active(&self, fallback: Option<&ModelFallback>) -> bool {
        fallback.is_some_and(|fallback| {
            !fallback.fallback_model.is_empty()
                && self.overridden_fallback != Some(fallback.source_seq)
        })
    }

    pub fn resolve(&self, fallback: Option<&ModelFallback>) -> ResolvedExecution {
        let defaults = self.defaults();
        let fallback_active = self.fallback_active(fallback);
        let fallback_model = fallback
            .filter(|_| fallback_active)
            .map(|fallback| fallback.fallback_model.clone());
        let raw_model = self
            .model
            .clone()
            .or_else(|| fallback_model.clone())
            .or_else(|| defaults.map(|defaults| defaults.model.clone()))
            .unwrap_or_default();
        let preferred_reasoning = self
            .reasoning
            .or_else(|| defaults.map(|defaults| defaults.reasoning_level))
            .filter(|level| level.is_known())
            .unwrap_or(ReasoningLevel::Medium);
        let provider = self.provider();
        let options = self.options.loaded();
        let verified = options.is_some_and(|options| options.model_load_error.is_none());
        let empty = Vec::new();
        let catalog = resolve_model_catalog_selection(
            options.map_or(&empty, |options| &options.models),
            options.map_or(&empty, |options| &options.selected_only_models),
            &raw_model,
            preferred_reasoning,
            provider,
            verified,
        );
        let model = match &fallback_model {
            Some(model) if self.model.is_none() => model.clone(),
            _ => catalog
                .active_model
                .as_ref()
                .map_or(catalog.selected_model.clone(), |active| {
                    active.model.clone()
                }),
        };
        let model_label = catalog
            .model_options
            .iter()
            .find(|option| option.value == model)
            .map_or_else(|| format_model_label(&model), |option| option.label.clone());

        let modes: Vec<PermissionMode> = provider
            .map(|provider| provider.capabilities.permission_modes.clone())
            .filter(|modes| !modes.is_empty())
            .unwrap_or_else(|| PermissionMode::KNOWN.to_vec());
        let ceiling = options
            .map(|options| options.permission_ceiling)
            .filter(|ceiling| ceiling.is_known())
            .unwrap_or(PermissionMode::Full);
        let ceiling_rank = permission_mode_rank(ceiling).unwrap_or(usize::MAX);
        let allowed: Vec<PermissionMode> = modes
            .iter()
            .copied()
            .filter(|mode| permission_mode_rank(*mode).is_some_and(|rank| rank <= ceiling_rank))
            .collect();
        let raw_permission = self
            .permission
            .or_else(|| defaults.map(|defaults| defaults.permission_mode))
            .unwrap_or(PermissionMode::Auto);
        let permission = resolve_permission_mode_selection(
            raw_permission,
            if allowed.is_empty() { &modes } else { &allowed },
        );
        let permission_options = PERMISSION_MODE_OPTIONS
            .iter()
            .filter(|option| modes.contains(&option.value))
            .map(|option| {
                let over =
                    permission_mode_rank(option.value).is_some_and(|rank| rank > ceiling_rank);
                PickerOption {
                    value: option.value,
                    label: option.label.to_owned(),
                    detail: Some(if over {
                        "Not allowed on this machine".to_owned()
                    } else {
                        option.description.to_owned()
                    }),
                    disabled: over,
                }
            })
            .collect();

        let supports_service_tier =
            provider.is_some_and(|provider| provider.capabilities.supports_service_tier);
        let tiers = service_tier_options(provider, catalog.active_model.as_ref());
        let raw_tier = self
            .service_tier
            .clone()
            .or_else(|| defaults.map(|defaults| defaults.service_tier.clone()))
            .filter(|tier| !tier.is_empty());
        let service_tier = if !supports_service_tier {
            None
        } else if tiers.is_empty() {
            Some(DEFAULT_SERVICE_TIER.to_owned())
        } else {
            raw_tier.map(|tier| {
                if tiers.contains(&tier) {
                    tier
                } else {
                    DEFAULT_SERVICE_TIER.to_owned()
                }
            })
        };

        let explicit = self.touched();
        let source = |present: bool, force: bool| {
            (present && (explicit || force)).then_some(ExecutionInputSource::Explicit)
        };
        let sources = ExistingThreadExecutionInputSources {
            model: source(!model.is_empty(), catalog.unavailable_model_recovery),
            service_tier: source(service_tier.is_some(), false),
            reasoning_level: source(true, false),
            permission_mode: source(true, false),
        };

        ResolvedExecution {
            model,
            model_label,
            active_model: catalog.active_model,
            reasoning: catalog.reasoning,
            reasoning_options: catalog.reasoning_options,
            permission,
            permission_options,
            model_options: catalog.model_options,
            service_tier,
            supports_service_tier,
            fallback_active,
            unavailable_model_recovery: catalog.unavailable_model_recovery,
            sources,
            defaults_state: self.defaults_state(),
        }
    }

    pub fn selection(&self, fallback: Option<&ModelFallback>) -> Option<ExecutionSelection> {
        self.defaults()?;
        let resolved = self.resolve(fallback);
        Some(ExecutionSelection {
            model: resolved.model,
            permission_mode: resolved.permission,
            reasoning_level: resolved.reasoning,
            service_tier: resolved.service_tier,
            supports_service_tier: resolved.supports_service_tier,
            sources: resolved.sources,
        })
    }

    pub fn select_model(&mut self, model: String, fallback: Option<&ModelFallback>) {
        if let Some(fallback) = fallback {
            self.overridden_fallback = Some(fallback.source_seq);
        }
        self.model = Some(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bb_wire::prompt::ThreadExecutionSource;
    use bb_wire::system::{ModelReasoningEffort, ProviderCapabilities, ProviderOptionDescriptor};

    fn model(id: &str, default: bool, efforts: &[ReasoningLevel]) -> AvailableModel {
        AvailableModel {
            id: id.into(),
            model: id.into(),
            display_name: id.into(),
            supported_reasoning_efforts: efforts
                .iter()
                .map(|level| ModelReasoningEffort {
                    reasoning_effort: *level,
                    description: String::new(),
                })
                .collect(),
            is_default: default,
            ..AvailableModel::default()
        }
    }

    fn state() -> ExecutionState {
        use ReasoningLevel as R;
        let mut state = ExecutionState::new("codex");
        state.options = Load::Loaded(SystemExecutionOptionsResponse {
            providers: vec![ProviderInfo {
                id: "codex".into(),
                capabilities: ProviderCapabilities {
                    permission_modes: vec![
                        PermissionMode::AcceptEdits,
                        PermissionMode::Auto,
                        PermissionMode::Full,
                    ],
                    supports_service_tier: true,
                    ..ProviderCapabilities::default()
                },
                reasoning_levels: Some(vec![ProviderOptionDescriptor {
                    id: "xhigh".into(),
                    label: "Turbo".into(),
                    description: None,
                }]),
                service_tiers: Some(vec![
                    ProviderOptionDescriptor {
                        id: "default".into(),
                        label: "Default".into(),
                        description: None,
                    },
                    ProviderOptionDescriptor {
                        id: "fast".into(),
                        label: "Fast".into(),
                        description: None,
                    },
                ]),
                ..ProviderInfo::default()
            }],
            permission_ceiling: PermissionMode::Auto,
            models: vec![
                model("gpt-5", true, &[R::Low, R::Medium, R::High]),
                model("gpt-5-mini", false, &[R::Low, R::Xhigh]),
            ],
            selected_only_models: vec![model("legacy-1", false, &[R::Medium])],
            model_load_error: None,
        });
        state.defaults = Load::Loaded(Some(ResolvedThreadExecutionOptions {
            model: "gpt-5".into(),
            service_tier: "fast".into(),
            reasoning_level: R::High,
            permission_mode: PermissionMode::Full,
            source: ThreadExecutionSource::TurnRequested,
            seq: None,
        }));
        state
    }

    #[test]
    fn untouched_selection_follows_defaults_clamped_to_ceiling() {
        let state = state();
        let resolved = state.resolve(None);
        assert_eq!(resolved.model, "gpt-5");
        assert_eq!(resolved.model_label, "GPT-5");
        assert_eq!(resolved.reasoning, ReasoningLevel::High);
        assert_eq!(resolved.permission, PermissionMode::Auto);
        assert!(resolved.permission_options[2].disabled);
        assert_eq!(resolved.service_tier.as_deref(), Some("fast"));
        assert_eq!(
            resolved.sources,
            ExistingThreadExecutionInputSources::default()
        );
        assert_eq!(resolved.model_options.len(), 3);
    }

    #[test]
    fn touching_a_field_marks_all_present_fields_explicit_and_reconciles_reasoning() {
        let mut state = state();
        state.select_model("gpt-5-mini".into(), None);
        let resolved = state.resolve(None);
        assert_eq!(resolved.reasoning, ReasoningLevel::Xhigh);
        assert_eq!(resolved.reasoning_options[1].label, "Turbo");
        assert_eq!(resolved.sources.model, Some(ExecutionInputSource::Explicit));
        assert_eq!(
            resolved.sources.permission_mode,
            Some(ExecutionInputSource::Explicit)
        );
        assert_eq!(
            resolved.sources.service_tier,
            Some(ExecutionInputSource::Explicit)
        );
        let selection = state.selection(None).unwrap();
        assert_eq!(selection.model, "gpt-5-mini");
    }

    #[test]
    fn unknown_model_recovers_to_default_explicitly() {
        let mut state = state();
        if let Load::Loaded(Some(defaults)) = &mut state.defaults {
            defaults.model = "retired".into();
        }
        let resolved = state.resolve(None);
        assert_eq!(resolved.model, "gpt-5");
        assert!(resolved.unavailable_model_recovery);
        assert_eq!(resolved.sources.model, Some(ExecutionInputSource::Explicit));
        assert_eq!(resolved.sources.reasoning_level, None);
    }

    #[test]
    fn model_fallback_wins_until_overridden() {
        let mut state = state();
        let fallback = ModelFallback {
            source_seq: 9,
            fallback_model: "gpt-5-mini".into(),
            original_model: "gpt-5".into(),
            ..ModelFallback::default()
        };
        assert_eq!(state.resolve(Some(&fallback)).model, "gpt-5-mini");
        state.select_model("gpt-5".into(), Some(&fallback));
        assert_eq!(state.resolve(Some(&fallback)).model, "gpt-5");
        assert!(!state.resolve(Some(&fallback)).fallback_active);
    }

    #[test]
    fn selection_requires_concrete_defaults() {
        let mut state = state();
        state.defaults = Load::Loaded(None);
        assert_eq!(state.defaults_state(), DefaultsState::Unavailable);
        assert!(state.selection(None).is_none());
        state.defaults = Load::Loading;
        assert_eq!(state.defaults_state(), DefaultsState::Loading);
        state.defaults = Load::Failed("boom".into());
        assert_eq!(state.defaults_state(), DefaultsState::Unavailable);
    }

    #[test]
    fn model_labels() {
        assert_eq!(format_model_label("gpt-5.1-codex"), "GPT-5.1-Codex");
        assert_eq!(format_model_label("Opus 5.5"), "Opus 5.5");
    }
}

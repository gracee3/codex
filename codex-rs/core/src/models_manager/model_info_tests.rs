use super::*;
use crate::config::test_config;
use crate::model_provider_info::WireApi;
use pretty_assertions::assert_eq;

#[test]
fn reasoning_summaries_override_true_enables_support() {
    let model = model_info_from_slug("unknown-model");
    let mut config = test_config();
    config.model_supports_reasoning_summaries = Some(true);

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.supports_reasoning_summaries = true;

    assert_eq!(updated, expected);
}

#[test]
fn reasoning_summaries_override_false_does_not_disable_support() {
    let mut model = model_info_from_slug("unknown-model");
    model.supports_reasoning_summaries = true;
    let mut config = test_config();
    config.model_supports_reasoning_summaries = Some(false);

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn reasoning_summaries_override_false_is_noop_when_model_is_false() {
    let model = model_info_from_slug("unknown-model");
    let mut config = test_config();
    config.model_supports_reasoning_summaries = Some(false);

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn vllm_gpt_oss_models_use_function_apply_patch() {
    let mut config = test_config();
    config.model_provider =
        crate::create_oss_provider_with_base_url("http://127.0.0.1:8000/v1", WireApi::Responses);
    config.model_provider.name = "vLLM".to_string();

    let mut gpt_oss = model_info_from_slug("gpt-oss-20b");
    gpt_oss.apply_patch_tool_type = None;

    let updated = with_config_overrides(gpt_oss, &config);

    assert_eq!(
        updated.apply_patch_tool_type,
        Some(codex_protocol::openai_models::ApplyPatchToolType::Function)
    );
}

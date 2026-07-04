use libplasma_pilot::{PolicyDecision, SafetyClass, ToolApprovalLevel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyConfig {
    pub default_observe: ToolApprovalLevel,
    pub default_control: ToolApprovalLevel,
    pub default_destructive_actions: ToolApprovalLevel,
    pub default_full_resolution_screenshot: ToolApprovalLevel,
    pub default_clipboard_read: ToolApprovalLevel,
    pub default_clipboard_write: ToolApprovalLevel,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            default_observe: ToolApprovalLevel::Allow,
            default_control: ToolApprovalLevel::Prompt,
            default_destructive_actions: ToolApprovalLevel::Prompt,
            default_full_resolution_screenshot: ToolApprovalLevel::Prompt,
            default_clipboard_read: ToolApprovalLevel::Prompt,
            default_clipboard_write: ToolApprovalLevel::Allow,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &PolicyConfig {
        &self.config
    }

    pub fn decide(&self, safety_class: &SafetyClass) -> PolicyDecision {
        let level = match safety_class {
            SafetyClass::Observe | SafetyClass::Policy => self.config.default_observe.clone(),
            SafetyClass::FullResolutionScreenshot => {
                self.config.default_full_resolution_screenshot.clone()
            }
            SafetyClass::ClipboardRead => self.config.default_clipboard_read.clone(),
            SafetyClass::ClipboardWrite => self.config.default_clipboard_write.clone(),
            SafetyClass::DestructiveAction => self.config.default_destructive_actions.clone(),
            SafetyClass::ControlPointer
            | SafetyClass::ControlKeyboard
            | SafetyClass::ControlSemantic => self.config.default_control.clone(),
        };

        PolicyDecision {
            level,
            reason: format!("default policy for {safety_class:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_is_allowed_by_default() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let decision = policy.decide(&SafetyClass::Observe);
        assert_eq!(decision.level, ToolApprovalLevel::Allow);
    }

    #[test]
    fn full_resolution_screenshot_prompts_by_default() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let decision = policy.decide(&SafetyClass::FullResolutionScreenshot);
        assert_eq!(decision.level, ToolApprovalLevel::Prompt);
    }

    #[test]
    fn destructive_actions_prompt_by_default() {
        let policy = PolicyEngine::new(PolicyConfig::default());
        let decision = policy.decide(&SafetyClass::DestructiveAction);
        assert_eq!(decision.level, ToolApprovalLevel::Prompt);
    }
}

//! Plan-mode tools: enter and exit plan mode.
//!
//! Translated from pi-mono:
//! - `tools/EnterPlanModeTool/EnterPlanModeTool.ts`
//! - `tools/ExitPlanModeTool/ExitPlanModeV2Tool.ts`
//!
//! Plan mode is a read-only exploration and planning phase.  The agent may
//! not edit files while in plan mode.  On exit the plan is presented to the
//! user for approval before any writes occur.

use crate::utils::permissions::engine::{PermissionDecision, ToolPermissionContext};
use crate::utils::permissions::mode::PermissionMode;

// ============================================================================
// Constants
// ============================================================================

pub const ENTER_PLAN_MODE_TOOL_NAME: &str = "EnterPlanMode";
pub const EXIT_PLAN_MODE_TOOL_NAME: &str = "ExitPlanMode";

// ============================================================================
// EnterPlanMode
// ============================================================================

/// Input for the EnterPlanMode tool (no parameters required).
#[derive(Debug, Clone, Default)]
pub struct EnterPlanModeInput;

/// Output returned by the EnterPlanMode tool.
#[derive(Debug, Clone)]
pub struct EnterPlanModeOutput {
    /// Confirmation message shown to the model.
    pub message: String,
}

/// The full instructions appended to the confirmation message when plan mode
/// interview phase is NOT enabled (standard path).
pub const ENTER_PLAN_MODE_STANDARD_INSTRUCTIONS: &str = "\n\nIn plan mode, you should:\n\
1. Thoroughly explore the codebase to understand existing patterns\n\
2. Identify similar features and architectural approaches\n\
3. Consider multiple approaches and their trade-offs\n\
4. Use AskUserQuestion if you need to clarify the approach\n\
5. Design a concrete implementation strategy\n\
6. When ready, use ExitPlanMode to present your plan for approval\n\
\n\
Remember: DO NOT write or edit any files yet. This is a read-only exploration and planning phase.";

/// Instructions used when the interview phase is enabled.
pub const ENTER_PLAN_MODE_INTERVIEW_INSTRUCTIONS: &str = "\n\nDO NOT write or edit any files except the plan file. \
Detailed workflow instructions will follow.";

/// Check whether plan mode can be entered and build the output.
///
/// Returns an error string if plan mode is unavailable in this context
/// (e.g. inside a sub-agent).
pub fn enter_plan_mode(
    ctx: &mut ToolPermissionContext,
    is_agent_context: bool,
    interview_phase: bool,
) -> Result<EnterPlanModeOutput, String> {
    if is_agent_context {
        return Err("EnterPlanMode tool cannot be used in agent contexts".to_owned());
    }

    // Save the current mode so we can restore it on exit.
    ctx.pre_plan_mode = Some(ctx.mode.clone());
    ctx.mode = PermissionMode::Plan;

    let base = "Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.";
    let instructions = if interview_phase {
        ENTER_PLAN_MODE_INTERVIEW_INSTRUCTIONS
    } else {
        ENTER_PLAN_MODE_STANDARD_INSTRUCTIONS
    };
    Ok(EnterPlanModeOutput {
        message: format!("{base}{instructions}"),
    })
}

// ============================================================================
// ExitPlanMode
// ============================================================================

/// How the plan exit should be handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanExitStrategy {
    /// Standard exit — restore the previous permission mode.
    Standard,
    /// Bypass permissions after plan approval (e.g. auto-approved).
    WithBypassPermissions,
}

/// Input for the ExitPlanMode tool.
#[derive(Debug, Clone)]
pub struct ExitPlanModeInput {
    /// The plan content to present for user approval.
    pub plan: String,
}

/// Output returned by the ExitPlanMode tool.
#[derive(Debug, Clone)]
pub struct ExitPlanModeOutput {
    /// Confirmation or status message.
    pub message: String,
    /// The restored permission mode after plan approval.
    pub restored_mode: PermissionMode,
}

/// Exit plan mode and restore the previous permission mode.
///
/// If the user approves the plan, the mode is restored to whatever it was
/// before plan mode was entered (or `Default` if unknown).
///
/// Returns an error string if plan mode is not currently active.
pub fn exit_plan_mode(
    ctx: &mut ToolPermissionContext,
    input: ExitPlanModeInput,
    strategy: PlanExitStrategy,
) -> Result<ExitPlanModeOutput, String> {
    if ctx.mode != PermissionMode::Plan {
        return Err("ExitPlanMode called while not in plan mode".to_owned());
    }

    // Restore the mode that was active before plan mode.
    let restored = match strategy {
        PlanExitStrategy::WithBypassPermissions => PermissionMode::BypassPermissions,
        PlanExitStrategy::Standard => ctx.pre_plan_mode.take().unwrap_or(PermissionMode::Default),
    };
    ctx.mode = restored.clone();

    let message = format!(
        "Plan approved. Exiting plan mode and restoring {} mode.\n\nPlan:\n{}",
        restored.title(),
        input.plan
    );

    Ok(ExitPlanModeOutput {
        message,
        restored_mode: restored,
    })
}

/// Check whether the ExitPlanMode tool is enabled.
pub fn is_exit_plan_mode_enabled() -> bool {
    true // TODO: check kairos channel availability when feature is implemented
}

/// Check whether the EnterPlanMode tool is enabled.
///
/// Mirrors `isEnabled()` from `EnterPlanModeTool.ts` — disabled when
/// `ExitPlanMode` is disabled (so the user cannot enter a mode they cannot leave).
pub fn is_enter_plan_mode_enabled() -> bool {
    true // TODO: check kairos channel availability when feature is implemented
}

/// Permission check for plan-mode tools.
///
/// Plan mode tools are always read-only; they never require permission prompts.
pub fn check_plan_mode_tool_permission(_ctx: &ToolPermissionContext) -> PermissionDecision {
    PermissionDecision::Allow { reason: None }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_plan_mode_transitions_context() {
        let mut ctx = ToolPermissionContext::new(PermissionMode::Default);
        let result = enter_plan_mode(&mut ctx, false, false);
        assert!(result.is_ok());
        assert_eq!(ctx.mode, PermissionMode::Plan);
        assert_eq!(ctx.pre_plan_mode, Some(PermissionMode::Default));
    }

    #[test]
    fn enter_plan_mode_fails_in_agent_context() {
        let mut ctx = ToolPermissionContext::default();
        let result = enter_plan_mode(&mut ctx, true, false);
        assert!(result.is_err());
    }

    #[test]
    fn exit_plan_mode_restores_previous_mode() {
        let mut ctx = ToolPermissionContext::new(PermissionMode::Default);
        enter_plan_mode(&mut ctx, false, false).unwrap();

        let input = ExitPlanModeInput {
            plan: "My implementation plan".to_owned(),
        };
        let output = exit_plan_mode(&mut ctx, input, PlanExitStrategy::Standard).unwrap();

        assert_eq!(output.restored_mode, PermissionMode::Default);
        assert_eq!(ctx.mode, PermissionMode::Default);
    }

    #[test]
    fn exit_plan_mode_with_bypass_sets_bypass_mode() {
        let mut ctx = ToolPermissionContext::new(PermissionMode::Default);
        enter_plan_mode(&mut ctx, false, false).unwrap();

        let input = ExitPlanModeInput {
            plan: "Plan".to_owned(),
        };
        let output =
            exit_plan_mode(&mut ctx, input, PlanExitStrategy::WithBypassPermissions).unwrap();

        assert_eq!(output.restored_mode, PermissionMode::BypassPermissions);
        assert_eq!(ctx.mode, PermissionMode::BypassPermissions);
    }

    #[test]
    fn exit_plan_mode_fails_when_not_in_plan_mode() {
        let mut ctx = ToolPermissionContext::new(PermissionMode::Default);
        let input = ExitPlanModeInput {
            plan: "Plan".to_owned(),
        };
        let result = exit_plan_mode(&mut ctx, input, PlanExitStrategy::Standard);
        assert!(result.is_err());
    }

    #[test]
    fn enter_plan_mode_output_contains_instructions() {
        let mut ctx = ToolPermissionContext::default();
        let output = enter_plan_mode(&mut ctx, false, false).unwrap();
        assert!(output.message.contains("ExitPlanMode"));
        assert!(output.message.contains("read-only"));
    }

    #[test]
    fn enter_plan_mode_interview_phase_different_instructions() {
        let standard_ctx = &mut ToolPermissionContext::default();
        let standard = enter_plan_mode(standard_ctx, false, false).unwrap().message;
        let interview_ctx = &mut ToolPermissionContext::default();
        let interview = enter_plan_mode(interview_ctx, false, true).unwrap().message;
        assert_ne!(standard, interview);
        assert!(interview.contains("plan file"));
    }

    #[test]
    fn tool_names_are_correct() {
        assert_eq!(ENTER_PLAN_MODE_TOOL_NAME, "EnterPlanMode");
        assert_eq!(EXIT_PLAN_MODE_TOOL_NAME, "ExitPlanMode");
    }

    #[test]
    fn plan_mode_tool_permission_always_allowed() {
        let ctx = ToolPermissionContext::default();
        let decision = check_plan_mode_tool_permission(&ctx);
        assert!(decision.is_allow());
    }
}

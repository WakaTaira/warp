use warpui::{App, SingletonEntity};

use super::register_orchestration_test_singletons;
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::connected_self_hosted_workers::ConnectedSelfHostedWorkersModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::LLMPreferences;
use crate::appearance::Appearance;

#[test]
fn orchestration_test_singletons_are_self_consistent() {
    App::test((), |mut app| async move {
        register_orchestration_test_singletons(&mut app);
        app.update(|ctx| {
            // Touch each registered accessor the orchestration card path
            // reads to prove the registered set is self-consistent.
            let _ = CloudAgentSettings::as_ref(ctx);
            let _ = Appearance::as_ref(ctx);
            let _ = LLMPreferences::as_ref(ctx);
            let _ = HarnessAvailabilityModel::as_ref(ctx);
            let _ = ConnectedSelfHostedWorkersModel::as_ref(ctx);
            let _ = BlocklistAIPermissions::as_ref(ctx);
            let _ = AIExecutionProfilesModel::as_ref(ctx);
        });
    });
}

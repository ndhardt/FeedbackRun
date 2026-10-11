use pipl::*;

const PF_PLUG_IN_VERSION: u16 = 13;
const PF_PLUG_IN_SUBVERS: u16 = 28;

#[rustfmt::skip]
fn main() {
    pipl::plugin_build(vec![
        Property::Kind(PIPLType::AEEffect),
        Property::Name("FeedbackRun"),
        Property::Category("FeedbackRun"),

        #[cfg(target_os = "windows")]
        Property::CodeWin64X86("EffectMain"),
        #[cfg(target_os = "macos")]
        Property::CodeMacIntel64("EffectMain"),
        #[cfg(target_os = "macos")]
        Property::CodeMacARM64("EffectMain"),

        Property::AE_PiPL_Version { major: 2, minor: 0 },
        Property::AE_Effect_Spec_Version { major: PF_PLUG_IN_VERSION, minor: PF_PLUG_IN_SUBVERS },
        Property::AE_Effect_Version {
            version: 10,
            subversion: 0,
            bugversion: 0,
            stage: Stage::Release,
            build: 1,
        },
        Property::AE_Effect_Info_Flags(0),
        Property::AE_Effect_Global_OutFlags(
            // We read the input at times other than `now`.
            OutFlags::WideTimeInput |
            OutFlags::NonParamVary |
            OutFlags::DeepColorAware
        ),
        Property::AE_Effect_Global_OutFlags_2(
            OutFlags2::SupportsSmartRender |
            OutFlags2::ParamGroupStartCollapsedFlag |
            // Lets AE track the over-time input checkouts and invalidate precisely.
            OutFlags2::AutomaticWideTimeInput |
            OutFlags2::FloatColorAware |
            // Feedback can reveal pixels outside the current frame's alpha.
            OutFlags2::RevealsZeroAlpha |
            // Paste reads the previous Copy state from our session cache.
            OutFlags2::IMixGuidDependencies |
            // The Paste/Copy pair consumes a previous Copy frame in order.
            OutFlags2::SupportsGetFlattenedSequenceData
        ),
        // Parameter indices changed in 0.7. A new match name prevents AE from
        // silently interpreting saved 0.6 sliders as different controls.
        Property::AE_Effect_Match_Name("FeedbackRun.2"),
        Property::AE_Reserved_Info(0),
        Property::AE_Effect_Support_URL("https://github.com/ndhardt/FeedbackRun"),
    ])
}

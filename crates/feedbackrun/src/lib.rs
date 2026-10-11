//! FeedbackRun: feedback through the layers between Paste and Copy.

use after_effects as ae;
use feedbackrun_engine as fx;
use std::{path::PathBuf, sync::Arc};

mod cache;
mod disk_cache;
mod pixels;

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
enum Params {
    Role,
    BlendMode,
    Decay,
    InputAmount,
    AnchorFrame,
    Reset,
    DecayGroupStart,
    DecayLink,
    DecayR,
    DecayG,
    DecayB,
    DecayGroupEnd,
    TransformGroupStart,
    FeedbackX,
    FeedbackY,
    FeedbackScale,
    FeedbackRotation,
    FeedbackBlur,
    TransformGroupEnd,
    OutputGroupStart,
    OutputGain,
    OutputTint,
    OutputClamp,
    OutputGroupEnd,
    CacheRevision,
    HistoryGroupStart,
    DelayFrames,
    HistoryFrames,
    FeedbackOn,
    HistoryGroupEnd,
    SwapOrder,
    DryWet,
    RestartHere,
    ClearTo,
    DiskCacheMode,
    DiskCacheFolder,
    DiskCachePathData,
    DiskCacheClear,
}

const COPY_CONTROLS: &[Params] = &[
    Params::BlendMode,
    Params::Decay,
    Params::InputAmount,
    Params::AnchorFrame,
    Params::DecayGroupStart,
    Params::DecayLink,
    Params::DecayR,
    Params::DecayG,
    Params::DecayB,
    Params::TransformGroupStart,
    Params::FeedbackX,
    Params::FeedbackY,
    Params::FeedbackScale,
    Params::FeedbackRotation,
    Params::FeedbackBlur,
    Params::OutputGroupStart,
    Params::OutputGain,
    Params::OutputTint,
    Params::OutputClamp,
    Params::HistoryGroupStart,
    Params::DelayFrames,
    Params::HistoryFrames,
    Params::FeedbackOn,
    Params::SwapOrder,
    Params::DryWet,
    Params::RestartHere,
    Params::ClearTo,
    Params::DiskCacheMode,
    Params::DiskCacheFolder,
    Params::DiskCacheClear,
];

const FEEDBACK_CONTROLS: &[Params] = &[
    Params::BlendMode,
    Params::Decay,
    Params::InputAmount,
    Params::AnchorFrame,
    Params::DecayLink,
    Params::DecayR,
    Params::DecayG,
    Params::DecayB,
    Params::FeedbackX,
    Params::FeedbackY,
    Params::FeedbackScale,
    Params::FeedbackRotation,
    Params::FeedbackBlur,
    Params::DelayFrames,
    Params::HistoryFrames,
    Params::FeedbackOn,
    Params::SwapOrder,
    Params::DryWet,
    Params::ClearTo,
    Params::DiskCacheMode,
];

#[derive(Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, PartialOrd)]
struct CacheFolder {
    path: PathBuf,
}

impl ae::pf::ArbitraryData<CacheFolder> for CacheFolder {
    fn interpolate(&self, _: &CacheFolder, _: f64) -> CacheFolder {
        self.clone()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiskCacheMode {
    Live,
    Record,
    Playback,
}

#[derive(Default)]
struct Plugin {
    cache: cache::SessionCache,
    aegp_id: ae::aegp::PluginId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Paste,
    Copy,
    Single,
}

impl Role {
    fn records_state(self) -> bool {
        self != Self::Paste
    }
}

struct PreRenderState {
    anchor: i32,
    first: i32,
    cached: Option<cache::Entry>,
    placeholder: bool,
    config: Option<cache::FrameConfig>,
    disk_playback: Option<(PathBuf, disk_cache::FrameKey)>,
}

ae::define_effect!(Plugin, (), Params);

fn role(params: &ae::Parameters<Params>) -> Result<Role, ae::Error> {
    Ok(match params.get(Params::Role)?.as_popup()?.value() {
        2 => Role::Copy,
        3 => Role::Single,
        _ => Role::Paste,
    })
}

fn disk_mode(params: &ae::Parameters<Params>) -> Result<DiskCacheMode, ae::Error> {
    Ok(
        match params.get(Params::DiskCacheMode)?.as_popup()?.value() {
            2 => DiskCacheMode::Record,
            3 => DiskCacheMode::Playback,
            _ => DiskCacheMode::Live,
        },
    )
}

fn cache_folder(params: &ae::Parameters<Params>) -> Result<PathBuf, ae::Error> {
    Ok(params
        .get(Params::DiskCachePathData)?
        .as_arbitrary()?
        .value::<CacheFolder>()?
        .path
        .clone())
}

fn disk_key(
    params: &ae::Parameters<Params>,
    in_data: &ae::InData,
    frame: i32,
    depth: i16,
    width: usize,
    height: usize,
    origin_x: i32,
    origin_y: i32,
) -> Result<disk_cache::FrameKey, ae::Error> {
    let downsample_x = in_data.downsample_x();
    let downsample_y = in_data.downsample_y();
    Ok(disk_cache::FrameKey {
        frame,
        revision: params.get(Params::CacheRevision)?.as_slider()?.value(),
        width,
        height,
        origin_x,
        origin_y,
        depth,
        time_step: in_data.time_step(),
        time_scale: in_data.time_scale(),
        downsample_x_num: downsample_x.num,
        downsample_x_den: downsample_x.den,
        downsample_y_num: downsample_y.num,
        downsample_y_den: downsample_y.den,
    })
}

fn bump_cache_revision(params: &mut ae::Parameters<Params>) -> Result<(), ae::Error> {
    let current = params.get(Params::CacheRevision)?.as_slider()?.value();
    set_cache_revision(params, (current + 1) % 1_000_001)
}

fn set_cache_revision(params: &mut ae::Parameters<Params>, value: i32) -> Result<(), ae::Error> {
    let mut revision = params.get_mut(Params::CacheRevision)?;
    revision.as_slider_mut()?.set_value(value);
    revision.set_value_changed();
    Ok(())
}

/// One pair per composition. The item ID prevents unrelated comps sharing history.
fn pair_id(in_data: &ae::InData) -> Result<i32, ae::Error> {
    let layer = ae::aegp::suites::PFInterface::new()?.effect_layer(in_data.effect_ref())?;
    let comp = ae::aegp::suites::Layer::new()?.layer_parent_comp(layer)?;
    let item = ae::aegp::suites::Comp::new()?.item_from_comp(comp)?;
    ae::aegp::suites::Item::new()?.item_id(item)
}

fn render_key(
    in_data: &ae::InData,
    id: i32,
    anchor: i32,
    depth: i16,
    x: i32,
    y: i32,
) -> cache::RenderKey {
    let mut key = cache::RenderKey::from_in_data(in_data, anchor, depth);
    key.instance = -id;
    key.rect_x = x;
    key.rect_y = y;
    key
}

fn frame_index(in_data: &ae::InData, time: i32) -> i32 {
    if in_data.time_step() <= 0 {
        return 0;
    }
    (time as f64 / in_data.time_step() as f64).floor() as i32
}

/// A finite history starts a fresh chain every N frames. This bounds the age
/// of any contribution without relying on an exponentially small decay.
fn segment_start(anchor: i32, frame: i32, history_frames: i32) -> i32 {
    if history_frames <= 0 || frame < anchor {
        return anchor;
    }
    let distance = i64::from(frame) - i64::from(anchor);
    let length = i64::from(history_frames);
    (i64::from(anchor) + distance / length * length) as i32
}

fn checkout_id(anchor: i32, frame: i32) -> Result<i32, ae::Error> {
    frame
        .checked_sub(anchor)
        .and_then(|n| n.checked_add(1))
        .filter(|&n| n > 0)
        .ok_or(ae::Error::InvalidParms)
}

fn state_receipt(
    in_data: &ae::InData,
    anchor: i32,
    end: i32,
) -> Result<ae::sys::PF_State, ae::Error> {
    let start = anchor
        .checked_mul(in_data.time_step())
        .ok_or(ae::Error::InvalidParms)?;
    let duration = end
        .checked_sub(anchor)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(in_data.time_step()))
        .ok_or(ae::Error::InvalidParms)?;
    ae::pf::suites::ParamUtils::new()?.current_state(
        in_data.effect_ref(),
        ae::pf::PARAM_INDEX_CHECK_ALL_HONOR_EXCLUDE,
        Some(ae::Time {
            value: start,
            scale: in_data.time_scale(),
        }),
        Some(ae::Time {
            value: duration,
            scale: in_data.time_scale(),
        }),
    )
}

fn feedback_step(
    params: &ae::Parameters<Params>,
    in_data: &ae::InData,
    frame: i32,
) -> Result<fx::FeedbackStep, ae::Error> {
    let time = frame
        .checked_mul(in_data.time_step())
        .ok_or(ae::Error::InvalidParms)?;
    let at = |id| {
        params.get_at(
            id,
            Some(time),
            Some(in_data.time_step()),
            Some(in_data.time_scale()),
        )
    };
    let master = at(Params::Decay)?.as_float_slider()?.value() as f32 / 100.0;
    let decay = if at(Params::DecayLink)?.as_checkbox()?.value() {
        [master; 3]
    } else {
        [
            at(Params::DecayR)?.as_float_slider()?.value() as f32 / 100.0,
            at(Params::DecayG)?.as_float_slider()?.value() as f32 / 100.0,
            at(Params::DecayB)?.as_float_slider()?.value() as f32 / 100.0,
        ]
    };
    let dsx = f32::from(in_data.downsample_x());
    let dsy = f32::from(in_data.downsample_y());
    Ok(fx::FeedbackStep {
        decay,
        mode: fx::BlendMode::from_popup(at(Params::BlendMode)?.as_popup()?.value()),
        opacity: at(Params::InputAmount)?.as_float_slider()?.value() as f32 / 100.0,
        swap_order: at(Params::SwapOrder)?.as_checkbox()?.value(),
        transform: fx::FeedbackTransform {
            x: at(Params::FeedbackX)?.as_float_slider()?.value() as f32 * dsx,
            y: at(Params::FeedbackY)?.as_float_slider()?.value() as f32 * dsy,
            scale: at(Params::FeedbackScale)?.as_float_slider()?.value() as f32 / 100.0,
            rotation_degrees: at(Params::FeedbackRotation)?.as_float_slider()?.value() as f32,
            blur_radius: ((at(Params::FeedbackBlur)?.as_slider()?.value().clamp(0, 8) as f32
                * dsx.min(dsy))
            .round() as i32)
                .clamp(0, 8) as u8,
        },
    })
}

fn feedback_config(
    params: &ae::Parameters<Params>,
    in_data: &ae::InData,
    frame: i32,
) -> Result<cache::FeedbackConfig, ae::Error> {
    let time = frame
        .checked_mul(in_data.time_step())
        .ok_or(ae::Error::InvalidParms)?;
    let at = |id| {
        params.get_at(
            id,
            Some(time),
            Some(in_data.time_step()),
            Some(in_data.time_scale()),
        )
    };
    Ok(cache::FeedbackConfig {
        step: feedback_step(params, in_data, frame)?,
        delay_frames: if role(params)? == Role::Single {
            1
        } else {
            at(Params::DelayFrames)?.as_slider()?.value().clamp(1, 120)
        },
        history_frames: params
            .get(Params::HistoryFrames)?
            .as_slider()?
            .value()
            .clamp(0, 512),
        enabled: at(Params::FeedbackOn)?.as_checkbox()?.value(),
        dry_wet: at(Params::DryWet)?.as_float_slider()?.value() as f32 / 100.0,
        clear_to_current: params.get(Params::ClearTo)?.as_popup()?.value() == 2,
    })
}

fn output_post(
    params: &ae::Parameters<Params>,
    in_data: &ae::InData,
    frame: i32,
) -> Result<fx::PostParams, ae::Error> {
    let time = frame
        .checked_mul(in_data.time_step())
        .ok_or(ae::Error::InvalidParms)?;
    let at = |id| {
        params.get_at(
            id,
            Some(time),
            Some(in_data.time_step()),
            Some(in_data.time_scale()),
        )
    };
    let tint = at(Params::OutputTint)?.as_color()?.value();
    Ok(fx::PostParams {
        gain: at(Params::OutputGain)?.as_float_slider()?.value() as f32 / 100.0,
        tint: [
            tint.red as f32 / 255.0,
            tint.green as f32 / 255.0,
            tint.blue as f32 / 255.0,
        ],
        mix: 0.0,
        clamp: at(Params::OutputClamp)?.as_checkbox()?.value(),
        show: fx::ShowMode::Result,
    })
}

impl AdobePluginGlobal for Plugin {
    fn params_setup(
        &self,
        params: &mut ae::Parameters<Params>,
        _: ae::InData,
        _: ae::OutData,
    ) -> Result<(), ae::Error> {
        params.add_with_flags(
            Params::Role,
            "Instance (Mode)",
            ae::PopupDef::setup(|f| {
                f.set_options(&[
                    "Paste (lower layer)",
                    "Copy (upper layer)",
                    "Single (Time Blend)",
                ]);
                f.set_default(1);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE
                | ae::ParamFlag::CANNOT_TIME_VARY
                | ae::ParamFlag::CANNOT_INTERP,
            ae::ParamUIFlags::empty(),
        )?;
        params.add_with_flags(
            Params::BlendMode,
            "Transfer / Operation",
            ae::PopupDef::setup(|f| {
                f.set_options(&fx::BlendMode::ITEMS.split('|').collect::<Vec<_>>());
                f.set_default(fx::BlendMode::Add as i32);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        params.add_with_flags(
            Params::Decay,
            "Accumulation (RGB + Alpha, %)",
            ae::FloatSliderDef::setup(|f| {
                f.set_valid_min(0.0);
                f.set_valid_max(100.0);
                f.set_slider_min(0.0);
                f.set_slider_max(100.0);
                f.set_default(100.0);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        params.add_with_flags(
            Params::InputAmount,
            "Input Amount (%)",
            ae::FloatSliderDef::setup(|f| {
                f.set_valid_min(0.0);
                f.set_valid_max(100.0);
                f.set_slider_min(0.0);
                f.set_slider_max(100.0);
                f.set_default(100.0);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        params.add_with_flags(
            Params::AnchorFrame,
            "Start Frame",
            ae::SliderDef::setup(|f| {
                f.set_valid_min(-100_000);
                f.set_valid_max(100_000);
                f.set_slider_min(0);
                f.set_slider_max(300);
                f.set_default(0);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        params.add_with_flags(
            Params::Reset,
            "Cache",
            ae::ButtonDef::setup(|f| {
                f.set_label("Reset cache and rebuild");
            }),
            ae::ParamFlag::SUPERVISE,
            ae::ParamUIFlags::INVISIBLE,
        )?;

        params.add_group(
            Params::DecayGroupStart,
            Params::DecayGroupEnd,
            "RGB Opacity",
            true,
            |params| {
                params.add_with_flags(
                    Params::DecayLink,
                    "Link RGB",
                    ae::CheckBoxDef::setup(|f| {
                        f.set_default(true);
                        f.set_label("Use Accumulation for all channels");
                        f.set_value(f.default());
                    }),
                    ae::ParamFlag::SUPERVISE,
                    ae::ParamUIFlags::empty(),
                )?;
                for (id, name) in [
                    (Params::DecayR, "Red"),
                    (Params::DecayG, "Green"),
                    (Params::DecayB, "Blue"),
                ] {
                    params.add_with_flags(
                        id,
                        name,
                        ae::FloatSliderDef::setup(|f| {
                            f.set_valid_min(0.0);
                            f.set_valid_max(100.0);
                            f.set_slider_min(0.0);
                            f.set_slider_max(100.0);
                            f.set_default(100.0);
                            f.set_value(f.default());
                        }),
                        ae::ParamFlag::SUPERVISE,
                        ae::ParamUIFlags::empty(),
                    )?;
                }
                Ok(())
            },
        )?;

        params.add_group(
            Params::TransformGroupStart,
            Params::TransformGroupEnd,
            "Transform (per frame)",
            true,
            |params| {
                for (id, name, min, max, default) in [
                    (
                        Params::FeedbackX,
                        "Translate X / frame",
                        -1000.0,
                        1000.0,
                        0.0,
                    ),
                    (
                        Params::FeedbackY,
                        "Translate Y / frame",
                        -1000.0,
                        1000.0,
                        0.0,
                    ),
                    (
                        Params::FeedbackScale,
                        "Scale / frame (%)",
                        1.0,
                        400.0,
                        100.0,
                    ),
                    (
                        Params::FeedbackRotation,
                        "Rotate / frame (deg)",
                        -360.0,
                        360.0,
                        0.0,
                    ),
                ] {
                    params.add_with_flags(
                        id,
                        name,
                        ae::FloatSliderDef::setup(|f| {
                            f.set_valid_min(min);
                            f.set_valid_max(max);
                            f.set_slider_min(min);
                            f.set_slider_max(max);
                            f.set_default(default);
                            f.set_value(f.default());
                        }),
                        ae::ParamFlag::SUPERVISE,
                        ae::ParamUIFlags::empty(),
                    )?;
                }
                params.add_with_flags(
                    Params::FeedbackBlur,
                    "Blur / frame (px)",
                    ae::SliderDef::setup(|f| {
                        f.set_valid_min(0);
                        f.set_valid_max(8);
                        f.set_slider_min(0);
                        f.set_slider_max(8);
                        f.set_default(0);
                        f.set_value(f.default());
                    }),
                    ae::ParamFlag::SUPERVISE,
                    ae::ParamUIFlags::empty(),
                )
            },
        )?;

        params.add_group(
            Params::OutputGroupStart,
            Params::OutputGroupEnd,
            "Copy Display",
            true,
            |params| {
                params.add_with_flags(
                    Params::OutputGain,
                    "Gain (%)",
                    ae::FloatSliderDef::setup(|f| {
                        f.set_valid_min(0.0);
                        f.set_valid_max(400.0);
                        f.set_slider_min(0.0);
                        f.set_slider_max(400.0);
                        f.set_default(100.0);
                        f.set_value(f.default());
                    }),
                    ae::ParamFlag::EXCLUDE_FROM_HAVE_INPUTS_CHANGED,
                    ae::ParamUIFlags::empty(),
                )?;
                params.add_with_flags(
                    Params::OutputTint,
                    "Tint",
                    ae::ColorDef::setup(|f| {
                        f.set_default(ae::Pixel8 {
                            red: 255,
                            green: 255,
                            blue: 255,
                            alpha: 255,
                        });
                        f.set_value(f.default());
                    }),
                    ae::ParamFlag::EXCLUDE_FROM_HAVE_INPUTS_CHANGED,
                    ae::ParamUIFlags::empty(),
                )?;
                params.add_with_flags(
                    Params::OutputClamp,
                    "Clamp",
                    ae::CheckBoxDef::setup(|f| {
                        f.set_default(false);
                        f.set_label("Clip to 0..1");
                        f.set_value(f.default());
                    }),
                    ae::ParamFlag::EXCLUDE_FROM_HAVE_INPUTS_CHANGED,
                    ae::ParamUIFlags::empty(),
                )
            },
        )?;
        params.add_with_flags(
            Params::CacheRevision,
            "Cache revision",
            ae::SliderDef::setup(|f| {
                f.set_valid_min(0);
                f.set_valid_max(1_000_000);
                f.set_slider_min(0);
                f.set_slider_max(1_000_000);
                f.set_default(0);
                f.set_value(f.default());
            }),
            ae::ParamFlag::CANNOT_TIME_VARY,
            ae::ParamUIFlags::INVISIBLE,
        )?;
        params.add_group(
            Params::HistoryGroupStart,
            Params::HistoryGroupEnd,
            "History",
            false,
            |params| {
                params.add_with_flags(
                    Params::DelayFrames,
                    "Delay (frames)",
                    ae::SliderDef::setup(|f| {
                        f.set_valid_min(1);
                        f.set_valid_max(120);
                        f.set_slider_min(1);
                        f.set_slider_max(120);
                        f.set_default(1);
                        f.set_value(f.default());
                    }),
                    ae::ParamFlag::SUPERVISE,
                    ae::ParamUIFlags::empty(),
                )?;
                params.add_with_flags(
                    Params::HistoryFrames,
                    "History limit (frames, 0 = unlimited)",
                    ae::SliderDef::setup(|f| {
                        f.set_valid_min(0);
                        f.set_valid_max(512);
                        f.set_slider_min(0);
                        f.set_slider_max(120);
                        f.set_default(0);
                        f.set_value(f.default());
                    }),
                    ae::ParamFlag::SUPERVISE | ae::ParamFlag::CANNOT_TIME_VARY,
                    ae::ParamUIFlags::empty(),
                )?;
                params.add_with_flags(
                    Params::FeedbackOn,
                    "Feedback On",
                    ae::CheckBoxDef::setup(|f| {
                        f.set_default(true);
                        f.set_label("Use saved frames; turn off to pass the input through");
                        f.set_value(f.default());
                    }),
                    ae::ParamFlag::SUPERVISE,
                    ae::ParamUIFlags::empty(),
                )
            },
        )?;
        params.add_with_flags(
            Params::SwapOrder,
            "Swap Operation Order",
            ae::CheckBoxDef::setup(|f| {
                f.set_default(false);
                f.set_label("Put feedback in front of the new frame");
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        params.add_with_flags(
            Params::DryWet,
            "Dry / Wet Mix (%)",
            ae::FloatSliderDef::setup(|f| {
                f.set_valid_min(0.0);
                f.set_valid_max(100.0);
                f.set_slider_min(0.0);
                f.set_slider_max(100.0);
                f.set_default(100.0);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        params.add(
            Params::RestartHere,
            "Clear / Reset Pulse (from playhead)",
            ae::ButtonDef::setup(|f| {
                f.set_label("Clear and restart here");
            }),
        )?;
        params.add_with_flags(
            Params::ClearTo,
            "Clear To (Blend only)",
            ae::PopupDef::setup(|f| {
                f.set_options(&["Transparent", "Current Frame"]);
                f.set_default(2);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE | ae::ParamFlag::CANNOT_TIME_VARY,
            ae::ParamUIFlags::empty(),
        )?;
        params.add_with_flags(
            Params::DiskCacheMode,
            "Disk Cache",
            ae::PopupDef::setup(|f| {
                f.set_options(&["Live", "Record to Disk", "Playback from Disk"]);
                f.set_default(1);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE | ae::ParamFlag::CANNOT_TIME_VARY,
            ae::ParamUIFlags::empty(),
        )?;
        params.add(
            Params::DiskCacheFolder,
            "Cache Folder",
            ae::ButtonDef::setup(|f| {
                f.set_label("Choose folder...");
            }),
        )?;
        params.add_with_flags(
            Params::DiskCachePathData,
            "Cache folder path",
            ae::ArbitraryDef::setup(|f| {
                f.set_default(CacheFolder::default()).unwrap();
            }),
            ae::ParamFlag::CANNOT_TIME_VARY | ae::ParamFlag::CANNOT_INTERP,
            ae::ParamUIFlags::INVISIBLE,
        )?;
        params.add(
            Params::DiskCacheClear,
            "Delete Disk Cache",
            ae::ButtonDef::setup(|f| {
                f.set_label("Delete cached frames");
            }),
        )?;
        Ok(())
    }

    fn handle_command(
        &mut self,
        cmd: ae::Command,
        in_data: ae::InData,
        mut out_data: ae::OutData,
        params: &mut ae::Parameters<Params>,
    ) -> Result<(), ae::Error> {
        match cmd {
            ae::Command::GlobalSetup => {
                self.aegp_id =
                    ae::aegp::suites::Utility::new()?.register_with_aegp("FeedbackRun")?;
            }
            ae::Command::About => {
                out_data.set_return_msg("FeedbackRun 1.0\r\rPaste: lower adjustment layer. Copy: upper adjustment layer.\rSingle: one-layer feedback. Disk Cache: record or play back native frames.");
            }
            ae::Command::ArbitraryCallback { mut extra } => {
                extra.dispatch::<CacheFolder, Params>(Params::DiskCachePathData)?;
            }
            ae::Command::UpdateParamsUi => {
                let linked = params.get(Params::DecayLink)?.as_checkbox()?.value();
                let mut ui = params.cloned();
                for id in [Params::DecayR, Params::DecayG, Params::DecayB] {
                    let mut control = ui.get_mut(id)?;
                    control.set_ui_flag(ae::ParamUIFlags::DISABLED, linked);
                    control.update_param_ui()?;
                }
                let selected_mode =
                    fx::BlendMode::from_popup(params.get(Params::BlendMode)?.as_popup()?.value());
                let is_blend = selected_mode == fx::BlendMode::Blend;
                let mut clear_to = ui.get_mut(Params::ClearTo)?;
                clear_to.set_ui_flag(ae::ParamUIFlags::DISABLED, !is_blend);
                clear_to.update_param_ui()?;
                let mut swap_order = ui.get_mut(Params::SwapOrder)?;
                swap_order.set_ui_flag(
                    ae::ParamUIFlags::DISABLED,
                    !selected_mode.is_order_sensitive(),
                );
                swap_order.update_param_ui()?;
                let mut delay = ui.get_mut(Params::DelayFrames)?;
                delay.set_ui_flag(ae::ParamUIFlags::DISABLED, role(params)? == Role::Single);
                delay.update_param_ui()?;
                let folder = cache_folder(params)?;
                let label = if folder.as_os_str().is_empty() {
                    "Choose folder...".to_owned()
                } else {
                    let name = folder.file_name().unwrap_or(folder.as_os_str());
                    format!(
                        "Change: {}",
                        name.to_string_lossy().chars().take(42).collect::<String>()
                    )
                };
                let mut folder_button = ui.get_mut(Params::DiskCacheFolder)?;
                folder_button.as_button_mut()?.set_label(&label);
                folder_button.update_param_ui()?;
                let mut clear_button = ui.get_mut(Params::DiskCacheClear)?;
                clear_button.set_ui_flag(ae::ParamUIFlags::DISABLED, folder.as_os_str().is_empty());
                clear_button.update_param_ui()?;
                let playback = disk_mode(params)? == DiskCacheMode::Playback;
                let mut reset_button = ui.get_mut(Params::RestartHere)?;
                reset_button.set_ui_flag(ae::ParamUIFlags::DISABLED, playback);
                reset_button.update_param_ui()?;
                if !in_data.is_premiere() && self.aegp_id != ae::aegp::PluginId::default() {
                    let hide = role(params)? == Role::Paste;
                    let me = in_data.effect().aegp_effect(self.aegp_id)?;
                    for &id in COPY_CONTROLS {
                        if let Some(index) = params.index(id) {
                            if let Ok(stream) = me.new_stream_by_index(self.aegp_id, index as _) {
                                stream.set_dynamic_stream_flag(
                                    ae::aegp::DynamicStreamFlags::Hidden,
                                    false,
                                    hide,
                                )?;
                            }
                        }
                    }
                }
            }
            ae::Command::UserChangedParam { param_index } => {
                let changed = params.type_at(param_index);
                if changed == Params::DiskCacheFolder && role(params)?.records_state() {
                    let current = cache_folder(params)?;
                    let mut dialog = rfd::FileDialog::new().set_title("FeedbackRun cache folder");
                    if !current.as_os_str().is_empty() && current.is_dir() {
                        dialog = dialog.set_directory(&current);
                    }
                    if let Some(selected) = dialog.pick_folder() {
                        let revision = match disk_cache::latest_revision(&selected) {
                            Ok(revision) if disk_mode(params)? == DiskCacheMode::Record => {
                                (revision.unwrap_or(0) + 1) % 1_000_001
                            }
                            Ok(revision) => revision.unwrap_or(0),
                            Err(error) => {
                                out_data.set_error_msg(&format!(
                                    "Cannot open FeedbackRun cache folder: {error}"
                                ));
                                return Err(ae::Error::InvalidParms);
                            }
                        };
                        set_cache_revision(params, revision)?;
                        params
                            .get_mut(Params::DiskCachePathData)?
                            .as_arbitrary_mut()?
                            .set_value(CacheFolder { path: selected })?;
                        self.cache.reset_pair(pair_id(&in_data)?);
                        out_data.set_out_flag(ae::OutFlags::RefreshUi, true);
                        out_data.set_force_rerender();
                    }
                } else if changed == Params::DiskCacheClear && role(params)?.records_state() {
                    let folder = cache_folder(params)?;
                    if folder.as_os_str().is_empty() {
                        out_data.set_error_msg("Choose a cache folder first.");
                        return Err(ae::Error::InvalidParms);
                    }
                    if let Err(error) = disk_cache::clear(&folder) {
                        out_data
                            .set_error_msg(&format!("Cannot delete FeedbackRun cache: {error}"));
                        return Err(ae::Error::InvalidParms);
                    }
                    bump_cache_revision(params)?;
                    self.cache.reset_pair(pair_id(&in_data)?);
                    out_data.set_force_rerender();
                } else if changed == Params::DiskCacheMode && role(params)?.records_state() {
                    if disk_mode(params)? == DiskCacheMode::Record {
                        bump_cache_revision(params)?;
                    }
                    self.cache.reset_pair(pair_id(&in_data)?);
                    out_data.set_out_flag(ae::OutFlags::RefreshUi, true);
                    out_data.set_force_rerender();
                } else if (changed == Params::Reset || changed == Params::RestartHere)
                    && role(params)?.records_state()
                {
                    if changed == Params::RestartHere {
                        let frame =
                            frame_index(&in_data, in_data.current_time()).clamp(-100_000, 100_000);
                        let mut start = params.get_mut(Params::AnchorFrame)?;
                        start.as_slider_mut()?.set_value(frame);
                        start.set_value_changed();
                    }
                    bump_cache_revision(params)?;
                    self.cache.reset_pair(pair_id(&in_data)?);
                    out_data.set_force_rerender();
                } else if changed == Params::Role {
                    self.cache.reset_pair(pair_id(&in_data)?);
                    out_data.set_force_rerender();
                } else if role(params)?.records_state() && FEEDBACK_CONTROLS.contains(&changed) {
                    self.cache.reset_pair(pair_id(&in_data)?);
                    out_data.set_force_rerender();
                }
            }
            ae::Command::SmartPreRender { mut extra } => {
                if in_data.time_step() <= 0 || in_data.time_scale() == 0 {
                    return Err(ae::Error::InvalidParms);
                }
                let mode = role(params)?;
                let req = extra.output_request();
                let end = frame_index(&in_data, in_data.current_time());
                let width = (req.rect.right - req.rect.left).max(0) as usize;
                let height = (req.rect.bottom - req.rect.top).max(0) as usize;
                let disk_mode = if mode.records_state() {
                    disk_mode(params)?
                } else {
                    DiskCacheMode::Live
                };
                if disk_mode == DiskCacheMode::Playback {
                    let folder = cache_folder(params)?;
                    if folder.as_os_str().is_empty() {
                        out_data.set_error_msg("Choose a FeedbackRun cache folder first.");
                        return Err(ae::Error::InvalidParms);
                    }
                    let key = disk_key(
                        params,
                        &in_data,
                        end,
                        extra.bit_depth(),
                        width,
                        height,
                        req.rect.left,
                        req.rect.top,
                    )?;
                    extra.callbacks().guid_mix_in_ptr(&[
                        disk_cache::fingerprint(&folder, key),
                        key.revision as u64,
                    ])?;
                    extra.set_result_rect(ae::Rect::from(req.rect));
                    extra.set_max_result_rect(ae::Rect::from(req.rect));
                    extra.set_pre_render_data(PreRenderState {
                        anchor: 0,
                        first: 0,
                        cached: None,
                        placeholder: false,
                        config: None,
                        disk_playback: Some((folder, key)),
                    });
                    return Ok(());
                }
                if disk_mode == DiskCacheMode::Record && !cache_folder(params)?.is_dir() {
                    out_data.set_error_msg("Choose an existing FeedbackRun cache folder first.");
                    return Err(ae::Error::InvalidParms);
                }
                let id = pair_id(&in_data)?;
                let mut cached = None;
                let mut config = None;
                let mut anchor = if mode.records_state() {
                    params.get(Params::AnchorFrame)?.as_slider()?.value()
                } else {
                    self.cache.anchor(id).unwrap_or(0)
                };
                let mut first = anchor;
                if mode.records_state() {
                    self.cache.set_anchor(id, anchor);
                    if end >= anchor {
                        let history_frames = params
                            .get(Params::HistoryFrames)?
                            .as_slider()?
                            .value()
                            .clamp(0, 512);
                        anchor = segment_start(anchor, end, history_frames);
                        first = anchor;
                        // Make every animated Copy value available to the
                        // matching Paste frame before AE traverses the stack.
                        let configs = (anchor..=end)
                            .map(|frame| Ok((frame, feedback_config(params, &in_data, frame)?)))
                            .collect::<Result<Vec<_>, ae::Error>>()?;
                        self.cache.publish_configs(id, &configs);
                        let key = render_key(
                            &in_data,
                            id,
                            anchor,
                            extra.bit_depth(),
                            req.rect.left,
                            req.rect.top,
                        );
                        let state_utils = ae::pf::suites::ParamUtils::new()?;
                        if let Some(candidate) = self.cache.candidates(key, end).into_iter().next()
                        {
                            let same_size = candidate.state.width() == width
                                && candidate.state.height() == height;
                            let same_inputs = same_size
                                && state_utils.are_states_identical(
                                    in_data.effect_ref(),
                                    &candidate.receipt,
                                    &state_receipt(&in_data, anchor, candidate.frame)?,
                                )?;
                            if same_inputs {
                                self.cache.touch(key, candidate.frame);
                                first = candidate.frame.saturating_add(1);
                                cached = Some(candidate);
                            } else {
                                self.cache.reset_pair(id);
                                self.cache.publish_configs(id, &configs);
                            }
                        }
                    }
                } else {
                    config = self.cache.frame_config(id, end);
                    if let Some(found) = config {
                        anchor = segment_start(found.anchor, end, found.feedback.history_frames);
                    }
                }

                let generation = self.cache.pair_generation(id);
                let config_serial = config.map_or(0, |found| found.serial);
                let previous_frame = config.and_then(|found| {
                    found
                        .feedback
                        .enabled
                        .then(|| end.saturating_sub(found.feedback.delay_frames))
                });
                let previous_serial =
                    if mode == Role::Paste && previous_frame.is_some_and(|frame| frame >= anchor) {
                        let key = render_key(
                            &in_data,
                            id,
                            anchor,
                            extra.bit_depth(),
                            req.rect.left,
                            req.rect.top,
                        );
                        self.cache
                            .exact_serial(key, previous_frame.unwrap(), width, height)
                    } else {
                        0
                    };
                extra
                    .callbacks()
                    .guid_mix_in_ptr(&[generation, config_serial, previous_serial])?;

                let mut placeholder = false;
                if mode.records_state() && end >= anchor && first <= end {
                    for frame in first..=end {
                        let time = frame
                            .checked_mul(in_data.time_step())
                            .ok_or(ae::Error::InvalidParms)?;
                        let result = extra.callbacks().checkout_layer(
                            0,
                            checkout_id(first, frame)?,
                            &req,
                            time,
                            in_data.time_step(),
                            in_data.time_scale(),
                        )?;
                        extra.union_result_rect(ae::Rect::from(result.result_rect));
                        extra.union_max_result_rect(ae::Rect::from(result.max_result_rect));
                    }
                } else {
                    let result = extra.callbacks().checkout_layer(
                        0,
                        1,
                        &req,
                        in_data.current_time(),
                        in_data.time_step(),
                        in_data.time_scale(),
                    )?;
                    extra.union_result_rect(ae::Rect::from(result.result_rect));
                    extra.union_max_result_rect(ae::Rect::from(result.max_result_rect));
                    placeholder = mode.records_state() && end >= anchor;
                }
                extra.set_result_rect(ae::Rect::from(req.rect));
                extra.set_max_result_rect(ae::Rect::from(req.rect));
                extra.set_pre_render_data(PreRenderState {
                    anchor,
                    first,
                    cached,
                    placeholder,
                    config,
                    disk_playback: None,
                });
            }
            ae::Command::SmartRender { extra } => {
                let cb = extra.callbacks();
                let pre = extra
                    .pre_render_data::<PreRenderState>()
                    .ok_or(ae::Error::InvalidParms)?;
                if let Some((ref folder, key)) = pre.disk_playback {
                    let Some(mut output) = cb.checkout_output()? else {
                        return Ok(());
                    };
                    let origin = output.origin();
                    if output.width() != key.width
                        || output.height() != key.height
                        || output.bit_depth() != key.depth
                        || origin.h != key.origin_x
                        || origin.v != key.origin_y
                    {
                        out_data
                            .set_error_msg("FeedbackRun cache does not match this preview size.");
                        return Err(ae::Error::InvalidParms);
                    }
                    let frame = match disk_cache::read(folder, key) {
                        Ok(Some(frame)) => frame,
                        Ok(None) => {
                            out_data.set_error_msg(
                                "FeedbackRun cache frame is missing. Record this frame first.",
                            );
                            return Err(ae::Error::InvalidParms);
                        }
                        Err(error) => {
                            out_data
                                .set_error_msg(&format!("Cannot read FeedbackRun cache: {error}"));
                            return Err(ae::Error::InvalidParms);
                        }
                    };
                    for y in 0..output.height() {
                        for x in 0..output.width() {
                            pixels::write_pixel(&mut output, x, y, key.depth, frame.get(x, y));
                        }
                    }
                    return Ok(());
                }
                let mode = role(params)?;
                let id = pair_id(&in_data)?;
                let end = frame_index(&in_data, in_data.current_time());
                let anchor = pre.anchor;
                let first_id = if mode.records_state() && end >= anchor && !pre.placeholder {
                    checkout_id(pre.first, pre.first)?
                } else {
                    1
                };
                let mut input = cb.checkout_layer_pixels(first_id as u32)?;
                let Some(mut output) = cb.checkout_output()? else {
                    cb.checkin_layer_pixels(first_id as u32)?;
                    return Ok(());
                };
                let depth = output.bit_depth();
                let origin = output.origin();
                let record_folder =
                    if mode.records_state() && disk_mode(params)? == DiskCacheMode::Record {
                        Some(cache_folder(params)?)
                    } else {
                        None
                    };

                if mode == Role::Paste {
                    // A direct request for Paste before Copy has published its
                    // settings is a transparent pass-through, never stale data.
                    if end < anchor || pre.config.is_none_or(|config| !config.feedback.enabled) {
                        if let Some(ref source) = input {
                            output.copy_from(source, None, None)?;
                        } else {
                            output.fill(None, None)?;
                        }
                    } else {
                        let settings = pre.config.as_ref().unwrap().feedback;
                        let step = settings.step;
                        let previous_frame = end.saturating_sub(settings.delay_frames);
                        let previous = if previous_frame >= anchor {
                            let key = render_key(&in_data, id, anchor, depth, origin.h, origin.v);
                            self.cache
                                .candidates(key, previous_frame)
                                .into_iter()
                                .find(|entry| {
                                    entry.frame == previous_frame
                                        && entry.state.width() == output.width()
                                        && entry.state.height() == output.height()
                                })
                                .map(|entry| {
                                    self.cache.touch(key, entry.frame);
                                    entry.state
                                })
                        } else {
                            None
                        };
                        let initialize_blend =
                            previous.is_none() && step.mode == fx::BlendMode::Blend;
                        let mut state = previous.map_or_else(
                            || fx::FrameBuffer::transparent(output.width(), output.height()),
                            |state| (*state).clone(),
                        );
                        if !initialize_blend {
                            state.step_with(
                                |x, y| {
                                    input.as_ref().map_or(fx::Rgba::TRANSPARENT, |layer| {
                                        pixels::read_pixel(layer, x, y)
                                    })
                                },
                                step,
                            );
                        }
                        for y in 0..output.height() {
                            for x in 0..output.width() {
                                let original =
                                    input.as_ref().map_or(fx::Rgba::TRANSPARENT, |layer| {
                                        pixels::read_pixel(layer, x, y)
                                    });
                                let feedback = if initialize_blend {
                                    if settings.clear_to_current {
                                        original
                                    } else {
                                        fx::Rgba::TRANSPARENT
                                    }
                                } else {
                                    state.get(x, y)
                                };
                                let mut color = original.lerp(feedback, settings.dry_wet);
                                color.a = color.a.clamp(0.0, 1.0);
                                pixels::write_pixel(&mut output, x, y, depth, color);
                            }
                        }
                    }
                    cb.checkin_layer_pixels(first_id as u32)?;
                    return Ok(());
                }

                if end < anchor {
                    if let Some(ref source) = input {
                        output.copy_from(source, None, None)?;
                    } else {
                        output.fill(None, None)?;
                    }
                    if let Some(folder) = &record_folder {
                        let frame = fx::FrameBuffer::from_pixels(
                            output.width(),
                            output.height(),
                            pixels::snapshot(Some(&output), output.width(), output.height()),
                        )
                        .ok_or(ae::Error::InvalidParms)?;
                        let key = disk_key(
                            params,
                            &in_data,
                            end,
                            depth,
                            output.width(),
                            output.height(),
                            origin.h,
                            origin.v,
                        )?;
                        if let Err(error) = disk_cache::write(folder, key, &frame) {
                            out_data
                                .set_error_msg(&format!("Cannot write FeedbackRun cache: {error}"));
                            return Err(ae::Error::InvalidParms);
                        }
                    }
                    cb.checkin_layer_pixels(first_id as u32)?;
                    return Ok(());
                }

                let key = render_key(&in_data, id, anchor, depth, origin.h, origin.v);
                let mut final_frame = pre.cached.as_ref().map(|entry| Arc::clone(&entry.state));
                let mut single_state = (mode == Role::Single).then(|| {
                    pre.cached.as_ref().map_or_else(
                        || fx::FrameBuffer::transparent(output.width(), output.height()),
                        |entry| (*entry.state).clone(),
                    )
                });
                if pre.first > end {
                    cb.checkin_layer_pixels(first_id as u32)?;
                }
                for frame in pre.first..=end {
                    let checkout = checkout_id(pre.first, frame)? as u32;
                    let layer = if checkout == first_id as u32 {
                        input.take()
                    } else {
                        cb.checkout_layer_pixels(checkout)?
                    };
                    let source = pixels::snapshot(layer.as_ref(), output.width(), output.height());
                    let framebuffer = if let Some(ref mut state) = single_state {
                        let settings = feedback_config(params, &in_data, frame)?;
                        if !settings.enabled
                            || (frame == anchor
                                && settings.step.mode == fx::BlendMode::Blend
                                && settings.clear_to_current)
                        {
                            *state = fx::FrameBuffer::from_pixels(
                                output.width(),
                                output.height(),
                                source,
                            )
                            .ok_or(ae::Error::InvalidParms)?;
                        } else if frame == anchor
                            && settings.step.mode == fx::BlendMode::Blend
                            && !settings.clear_to_current
                        {
                            *state = fx::FrameBuffer::transparent(output.width(), output.height());
                        } else {
                            state.step(Some(&source), settings.step);
                        }
                        Arc::new(state.clone())
                    } else {
                        Arc::new(
                            fx::FrameBuffer::from_pixels(output.width(), output.height(), source)
                                .ok_or(ae::Error::InvalidParms)?,
                        )
                    };
                    cb.checkin_layer_pixels(checkout)?;
                    let receipt = state_receipt(&in_data, anchor, frame)?;
                    self.cache.insert(cache::Entry::new(
                        key,
                        frame,
                        receipt,
                        Arc::clone(&framebuffer),
                    ));
                    if let Some(folder) = &record_folder
                        && frame < end
                    {
                        let post = output_post(params, &in_data, frame)?;
                        let pixels = framebuffer
                            .pixels()
                            .iter()
                            .map(|&pixel| fx::apply_post(pixel, fx::Rgba::TRANSPARENT, &post))
                            .collect();
                        let disk_frame =
                            fx::FrameBuffer::from_pixels(output.width(), output.height(), pixels)
                                .ok_or(ae::Error::InvalidParms)?;
                        let disk_key = disk_key(
                            params,
                            &in_data,
                            frame,
                            depth,
                            output.width(),
                            output.height(),
                            origin.h,
                            origin.v,
                        )?;
                        if let Err(error) = disk_cache::write(folder, disk_key, &disk_frame) {
                            out_data
                                .set_error_msg(&format!("Cannot write FeedbackRun cache: {error}"));
                            return Err(ae::Error::InvalidParms);
                        }
                    }
                    final_frame = Some(framebuffer);
                }
                let final_frame = final_frame.ok_or(ae::Error::InvalidParms)?;
                let post = output_post(params, &in_data, end)?;
                for y in 0..output.height() {
                    for x in 0..output.width() {
                        pixels::write_pixel(
                            &mut output,
                            x,
                            y,
                            depth,
                            fx::apply_post(final_frame.get(x, y), fx::Rgba::TRANSPARENT, &post),
                        );
                    }
                }
                if let Some(folder) = &record_folder {
                    let frame = fx::FrameBuffer::from_pixels(
                        output.width(),
                        output.height(),
                        pixels::snapshot(Some(&output), output.width(), output.height()),
                    )
                    .ok_or(ae::Error::InvalidParms)?;
                    let key = disk_key(
                        params,
                        &in_data,
                        end,
                        depth,
                        output.width(),
                        output.height(),
                        origin.h,
                        origin.v,
                    )?;
                    if let Err(error) = disk_cache::write(folder, key, &frame) {
                        out_data.set_error_msg(&format!("Cannot write FeedbackRun cache: {error}"));
                        return Err(ae::Error::InvalidParms);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

//! Expansão de comandos em ops primitivas (determinística sobre o snapshot atual).

mod check;
mod clips;
mod keyframes;
mod nested;
mod ripple;
mod structure;

use crate::command::Command;
use crate::ctx::{CommandOutput, Ctx};
use crate::error::Result;

pub(crate) use nested::reconcile_follow;

pub(crate) fn execute_command(ctx: &mut Ctx, command: &Command) -> Result<CommandOutput> {
    match command {
        Command::RegisterAsset { asset } => structure::register_asset(ctx, asset),
        Command::DeleteAsset { asset } => structure::delete_asset(ctx, asset),
        Command::CreateSequence {
            id,
            name,
            frame_rate,
            sample_rate,
        } => structure::create_sequence(ctx, id.as_ref(), name, *frame_rate, *sample_rate),
        Command::AddTrack {
            sequence,
            id,
            kind,
            name,
            role,
            magnetic,
            index,
        } => structure::add_track(
            ctx,
            sequence,
            id.as_ref(),
            *kind,
            name.as_deref(),
            role.as_ref(),
            *magnetic,
            *index,
        ),
        Command::SetTrackFlags {
            track,
            locked,
            hidden,
            muted,
            solo,
            magnetic,
            sync_lock,
            group,
            clear_group,
            compact,
        } => structure::set_track_flags(
            ctx,
            track,
            &structure::TrackFlags {
                locked: *locked,
                hidden: *hidden,
                muted: *muted,
                solo: *solo,
                magnetic: *magnetic,
                sync_lock: *sync_lock,
                group: group.as_deref(),
                clear_group: *clear_group,
                compact: *compact,
            },
        ),
        Command::DeleteTrack { track } => structure::delete_track(ctx, track),
        Command::DeleteSequence { sequence } => structure::delete_sequence(ctx, sequence),
        Command::RenameSequence { sequence, name } => {
            structure::rename_sequence(ctx, sequence, name)
        }
        Command::InsertNested {
            track,
            start,
            sequence,
            id,
            name,
            duration,
            source_in,
            follow_length,
            split_at_insert,
            split_new_id,
        } => nested::insert_nested(
            ctx,
            track,
            *start,
            sequence,
            id.as_ref(),
            name,
            *duration,
            *source_in,
            *follow_length,
            *split_at_insert,
            split_new_id.as_ref(),
        ),
        Command::SetNestedTarget { clip, sequence } => {
            nested::set_nested_target(ctx, clip, sequence)
        }
        Command::SetFollowLength {
            clip,
            follow_length,
        } => nested::set_follow_length(ctx, clip, *follow_length),
        Command::AddMarker {
            sequence,
            id,
            time,
            label,
        } => structure::add_marker(ctx, sequence, id.as_ref(), *time, label),
        Command::MoveMarker {
            sequence,
            marker,
            time,
        } => structure::move_marker(ctx, sequence, marker, *time),
        Command::DeleteMarker { sequence, marker } => {
            structure::delete_marker(ctx, sequence, marker)
        }
        Command::InsertClip {
            track,
            start,
            clip,
            split_at_insert,
            split_new_id,
        } => clips::insert_clip(
            ctx,
            track,
            *start,
            clip,
            *split_at_insert,
            split_new_id.as_ref(),
        ),
        Command::MoveClips { moves } => clips::move_clips(ctx, moves),
        Command::DeleteClip {
            clip,
            ripple,
            scope,
        } => clips::delete_clip(ctx, clip, *ripple, scope),
        Command::TrimClip {
            clip,
            edge,
            to,
            ripple,
            scope,
        } => clips::trim_clip(ctx, clip, *edge, *to, *ripple, scope),
        Command::SplitClip { clip, at, new_id } => {
            clips::split_clip(ctx, clip, *at, new_id.clone())
        }
        Command::SetClipSpeed {
            clip,
            speed,
            ripple,
            scope,
        } => clips::set_clip_speed(ctx, clip, *speed, *ripple, scope),
        Command::SetProperty { clip, prop, value } => {
            keyframes::set_property(ctx, clip, prop, *value)
        }
        Command::AddKeyframe {
            clip,
            prop,
            at,
            value,
            interp,
        } => keyframes::add_keyframe(ctx, clip, prop, *at, *value, *interp),
        Command::MoveKeyframe {
            clip,
            prop,
            from,
            to,
        } => keyframes::move_keyframe(ctx, clip, prop, *from, *to),
        Command::DeleteKeyframe { clip, prop, at } => {
            keyframes::delete_keyframe(ctx, clip, prop, *at)
        }
        Command::SetKeyframeInterp {
            clip,
            prop,
            at,
            interp,
        } => keyframes::set_keyframe_interp(ctx, clip, prop, *at, *interp),
    }
}

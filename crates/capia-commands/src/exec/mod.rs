//! Expansão de comandos em ops primitivas (determinística sobre o snapshot atual).

mod check;
mod clips;
mod compose;
mod editor;
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
        Command::UpdateAsset { asset } => structure::update_asset(ctx, asset),
        Command::DuplicateSequence {
            source,
            new_sequence,
            name,
            deep,
        } => {
            compose::duplicate_sequence(ctx, source, new_sequence.as_ref(), name.as_deref(), *deep)
        }
        Command::MakeUnique {
            clip,
            new_sequence,
            name,
            deep,
        } => compose::make_unique(ctx, clip, new_sequence.as_ref(), name.as_deref(), *deep),
        Command::FlattenNested { clip, prefix } => {
            compose::flatten_nested(ctx, clip, prefix.as_deref())
        }
        Command::CreateNestedFromSelection {
            clips,
            new_sequence,
            name,
            clip_id,
            track,
            follow_length,
        } => compose::create_nested_from_selection(
            ctx,
            clips,
            new_sequence.as_ref(),
            name.as_deref(),
            clip_id.as_ref(),
            track.as_ref(),
            *follow_length,
        ),
        Command::GenerateVariants { template, variants } => {
            compose::generate_variants(ctx, template, variants)
        }
        Command::CreateSequence {
            id,
            name,
            frame_rate,
            sample_rate,
            width,
            height,
            folder,
        } => structure::create_sequence(
            ctx,
            id.as_ref(),
            name,
            *frame_rate,
            *sample_rate,
            (*width, *height),
            folder.as_ref(),
        ),
        Command::SetSequenceFormat {
            sequence,
            width,
            height,
        } => editor::set_sequence_format(ctx, sequence, *width, *height),
        Command::SetSequenceFolder { sequence, folder } => {
            editor::set_sequence_folder(ctx, sequence, folder.as_ref())
        }
        Command::CreateFolder { id, name, parent } => {
            editor::create_folder(ctx, id.as_ref(), name, parent.as_ref())
        }
        Command::RenameFolder { folder, name } => editor::rename_folder(ctx, folder, name),
        Command::MoveFolder { folder, parent } => editor::move_folder(ctx, folder, parent.as_ref()),
        Command::DeleteFolder { folder } => editor::delete_folder(ctx, folder),
        Command::CreateDeliverable {
            id,
            name,
            sequence,
            preset,
            path,
            width,
            height,
        } => editor::create_deliverable(
            ctx,
            id.as_ref(),
            name,
            sequence,
            preset,
            path,
            *width,
            *height,
        ),
        Command::UpdateDeliverable { deliverable } => editor::update_deliverable(ctx, deliverable),
        Command::DeleteDeliverable { deliverable } => editor::delete_deliverable(ctx, deliverable),
        Command::RenameClip { clip, name } => editor::rename_clip(ctx, clip, name),
        Command::SetClipEnabled { clip, enabled } => editor::set_clip_enabled(ctx, clip, *enabled),
        Command::SetText { clip, text, style } => {
            editor::set_text(ctx, clip, text.as_deref(), style.as_ref())
        }
        Command::GroupClips { clips, group } => editor::group_clips(ctx, clips, group.as_deref()),
        Command::Ungroup { clips } => editor::ungroup(ctx, clips),
        Command::SetTransition { clip, transition } => {
            editor::set_transition(ctx, clip, transition.as_ref())
        }
        Command::DetachAudio {
            clip,
            audio_track,
            audio_clip_id,
        } => editor::detach_audio(ctx, clip, audio_track.as_ref(), audio_clip_id.as_ref()),
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

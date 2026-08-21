//! Turning local rows into the changes the archive accepts, and back.
//!
//! Kept apart from the database and the network so the mapping can be tested
//! without either. It is also the place a mistake is least visible: a field
//! dropped here does not fail, it simply stops arriving on the other device,
//! and nothing says so.

use serde::{Deserialize, Serialize};

/// The kinds the archive knows. Spelled exactly as the service's `ChildKind`
/// and change tags — the two are one wire contract, and a mismatch would be
/// rejected as an unknown kind rather than silently ignored.
pub const KIND_NOTE: &str = "note";
pub const KIND_SUMMARY: &str = "summary";
pub const KIND_ACTION_ITEM: &str = "actionItem";
pub const KIND_TAG: &str = "tag";
pub const KIND_LINK: &str = "link";
pub const KIND_TRANSCRIPT_VERSION: &str = "transcriptVersion";

/// Whether a kind is a child of its note, as opposed to the note itself or a
/// link in the chain. The archive authorizes children through their parent, so
/// this decides which shape a change takes on the wire.
pub fn is_child(kind: &str) -> bool {
    matches!(
        kind,
        KIND_SUMMARY | KIND_ACTION_ITEM | KIND_TAG | KIND_LINK
    )
}

/// A change ready to send.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OutgoingChange {
    Note {
        #[serde(rename = "clientChangeId")]
        client_change_id: String,
        kind: &'static str,
        #[serde(rename = "noteId")]
        note_id: String,
        deleted: bool,
        #[serde(rename = "updatedAt")]
        updated_at: String,
        payload: serde_json::Value,
    },
    Child {
        #[serde(rename = "clientChangeId")]
        client_change_id: String,
        kind: &'static str,
        #[serde(rename = "noteId")]
        note_id: String,
        #[serde(rename = "childKind")]
        child_kind: String,
        #[serde(rename = "entityId")]
        entity_id: String,
        deleted: bool,
        #[serde(rename = "updatedAt")]
        updated_at: String,
        payload: serde_json::Value,
    },
    TranscriptVersion {
        #[serde(rename = "clientChangeId")]
        client_change_id: String,
        kind: &'static str,
        #[serde(rename = "noteId")]
        note_id: String,
        version: serde_json::Value,
    },
}

/// What the archive says happened to a change.
///
/// `Stale` and `Conflict` are not errors. They are the archive telling this
/// device that it lost a race and exactly what it lost to, which is the only
/// way two devices ever agree on anything.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ChangeOutcome {
    Applied {
        seq: i64,
    },
    Stale {
        winner: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    Conflict {
        version: i32,
        content_hash: String,
    },
    Rejected {
        reason: String,
    },
}

impl ChangeOutcome {
    /// Whether the queued change can be cleared.
    ///
    /// A rejection clears too. The archive rejects a change when retrying it
    /// unchanged cannot help — an unknown note, a hash that does not describe
    /// its content — so keeping it queued would retry it on every sync for
    /// ever, and hold up nothing but itself.
    pub fn is_settled(&self) -> bool {
        match self {
            ChangeOutcome::Applied { .. }
            | ChangeOutcome::Stale { .. }
            | ChangeOutcome::Rejected { .. } => true,
            // A conflict is the one case where more work is needed here: the
            // chain has to be re-based onto the winner before this content can
            // be appended again.
            ChangeOutcome::Conflict { .. } => false,
        }
    }
}

/// The payload for a note.
///
/// Only what another device needs to show the note. Audio paths are local to
/// one machine and recordings do not sync, so a path sent here would name a
/// file that does not exist wherever it arrived.
pub fn note_payload(
    title: &str,
    description: Option<&str>,
    participants: Option<&str>,
    started_at: &str,
    ended_at: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "description": description,
        "participants": participants,
        "startedAt": started_at,
        "endedAt": ended_at,
    })
}

pub fn summary_payload(summary_type: &str, content: &str, created_at: &str) -> serde_json::Value {
    serde_json::json!({
        "summaryType": summary_type,
        "content": content,
        "createdAt": created_at,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn action_item_payload(
    text: &str,
    description: Option<&str>,
    assignee: Option<&str>,
    due_date: Option<&str>,
    done: bool,
    sort_order: i64,
    parent_stable_id: Option<&str>,
    created_at: &str,
) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        "description": description,
        "assignee": assignee,
        "dueDate": due_date,
        "done": done,
        "sortOrder": sort_order,
        // The parent's stable id, never its row id: row ids are local, so a
        // sub-task would attach itself to whatever happened to hold that
        // number on the other device.
        "parentStableId": parent_stable_id,
        "createdAt": created_at,
    })
}

pub fn tag_payload(name: &str, color: Option<&str>) -> serde_json::Value {
    serde_json::json!({ "name": name, "color": color })
}

pub fn link_payload(target_title: &str, target_note_id: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "targetTitle": target_title,
        // Only meaningful once the note it points at has synced. Absent, the
        // other device still has the title and can resolve the link itself.
        "targetNoteId": target_note_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_are_the_ones_authorized_through_their_note() {
        for kind in [KIND_SUMMARY, KIND_ACTION_ITEM, KIND_TAG, KIND_LINK] {
            assert!(is_child(kind), "{kind}");
        }
        // The note is the thing that carries ownership, and a chain link is
        // verified rather than stored, so neither travels as a child.
        assert!(!is_child(KIND_NOTE));
        assert!(!is_child(KIND_TRANSCRIPT_VERSION));
    }

    #[test]
    fn an_unknown_kind_is_not_a_child() {
        assert!(!is_child("attachment"));
    }

    #[test]
    fn a_note_change_serializes_the_way_the_archive_reads_it() {
        let change = OutgoingChange::Note {
            client_change_id: "c1".into(),
            kind: KIND_NOTE,
            note_id: "n1".into(),
            deleted: false,
            updated_at: "2026-08-11T10:00:00Z".into(),
            payload: note_payload("Weekly", None, None, "2026-08-11T09:00:00Z", None),
        };
        let v = serde_json::to_value(&change).unwrap();
        assert_eq!(v["kind"], "note");
        assert_eq!(v["clientChangeId"], "c1");
        assert_eq!(v["noteId"], "n1");
        assert_eq!(v["payload"]["title"], "Weekly");
    }

    #[test]
    fn a_child_change_names_its_kind_separately() {
        // The tag is `child`; what kind of child it is rides alongside, because
        // the archive stores all four in one table and authorizes them all
        // through the same note.
        let change = OutgoingChange::Child {
            client_change_id: "c2".into(),
            kind: "child",
            note_id: "n1".into(),
            child_kind: KIND_TAG.into(),
            entity_id: "standup".into(),
            deleted: false,
            updated_at: "2026-08-11T10:00:00Z".into(),
            payload: tag_payload("standup", Some("#f00")),
        };
        let v = serde_json::to_value(&change).unwrap();
        assert_eq!(v["kind"], "child");
        assert_eq!(v["childKind"], "tag");
        assert_eq!(v["entityId"], "standup");
    }

    #[test]
    fn a_note_payload_carries_no_audio_path() {
        // Recordings do not sync, so a path would name a file that does not
        // exist wherever it arrived.
        let v = note_payload("Weekly", None, None, "t", None);
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert!(!keys.iter().any(|k| k.contains("audio") || k.contains("path")));
    }

    #[test]
    fn an_action_item_points_at_its_parent_by_stable_id() {
        // Row ids are local. A sub-task carrying one would attach itself to
        // whatever happened to hold that number on the other device.
        let v = action_item_payload("Follow up", None, None, None, false, 0, Some("ai-parent"), "t");
        assert_eq!(v["parentStableId"], "ai-parent");
        assert!(v.get("parentId").is_none());
    }

    #[test]
    fn outcomes_parse_from_what_the_service_sends() {
        let applied: ChangeOutcome =
            serde_json::from_str(r#"{"status":"applied","seq":7}"#).unwrap();
        assert_eq!(applied, ChangeOutcome::Applied { seq: 7 });

        let conflict: ChangeOutcome =
            serde_json::from_str(r#"{"status":"conflict","version":2,"contentHash":"abc"}"#)
                .unwrap();
        assert_eq!(
            conflict,
            ChangeOutcome::Conflict {
                version: 2,
                content_hash: "abc".into()
            }
        );
    }

    #[test]
    fn everything_but_a_conflict_settles_the_queued_change() {
        assert!(ChangeOutcome::Applied { seq: 1 }.is_settled());
        assert!(ChangeOutcome::Stale {
            winner: serde_json::Value::Null
        }
        .is_settled());
        // A rejection settles too: the archive rejects only when retrying
        // unchanged cannot help, so keeping it queued would retry it for ever.
        assert!(ChangeOutcome::Rejected {
            reason: "unknown note".into()
        }
        .is_settled());
    }

    #[test]
    fn a_conflict_stays_queued_because_it_needs_a_rebase() {
        assert!(!ChangeOutcome::Conflict {
            version: 2,
            content_hash: "abc".into()
        }
        .is_settled());
    }
}

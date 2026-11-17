//! Account-local transient drafts. No drafts or signer secrets are written to disk.
use crate::core::Post;
use std::collections::BTreeMap;

#[derive(Default)]
struct DraftState {
    text: String,
    reply: Option<Post>,
}
#[derive(Default)]
pub struct DraftSessions {
    drafts: BTreeMap<String, DraftState>,
}
impl DraftSessions {
    pub fn switch(&mut self, from: &str, to: &str, current: &mut String, reply: &mut Option<Post>) {
        if from == to {
            return;
        }
        if !from.is_empty() {
            self.drafts.insert(
                from.to_owned(),
                DraftState {
                    text: std::mem::take(current),
                    reply: reply.take(),
                },
            );
        } else {
            current.clear();
            *reply = None;
        }
        let next = self.drafts.remove(to).unwrap_or_default();
        *current = next.text;
        *reply = next.reply;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn target(id: &str) -> Post {
        Post {
            id: id.into(),
            author: "author".into(),
            created_at: 1,
            content: "parent".into(),
            event_json: "{}".into(),
        }
    }
    #[test]
    fn drafts_and_reply_targets_never_move_to_a_different_account() {
        let mut drafts = DraftSessions::default();
        let mut current = "draft for A".to_owned();
        let mut reply = Some(target("A"));
        drafts.switch("a", "b", &mut current, &mut reply);
        assert_eq!(current, "");
        assert!(reply.is_none());
        current = "draft for B".into();
        reply = Some(target("B"));
        drafts.switch("b", "a", &mut current, &mut reply);
        assert_eq!(current, "draft for A");
        assert_eq!(reply.as_ref().unwrap().id, "A");
        drafts.switch("a", "b", &mut current, &mut reply);
        assert_eq!(current, "draft for B");
        assert_eq!(reply.as_ref().unwrap().id, "B");
        drafts.switch("b", "b", &mut current, &mut reply);
        assert_eq!(current, "draft for B");
    }
}

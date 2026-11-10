//! V103: exercise the production focus policy against the real AccessKit consumer.
#[path = "../third_party/slint/i-slint-backend-winit/accesskit_focus.rs"]
mod accesskit_focus;

use accesskit::{Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use accesskit_consumer::{NodeRef as ConsumerNode, TreeChangeHandler};

const ROOT: NodeId = NodeId(1);
const OLD: NodeId = NodeId(2);
const NEW: NodeId = NodeId(3);

struct Changes;
impl TreeChangeHandler for Changes {
    fn node_added(&mut self, _: &ConsumerNode) {}
    fn node_updated(&mut self, _: &ConsumerNode, _: &ConsumerNode) {}
    fn focus_moved(&mut self, _: Option<&ConsumerNode>, _: Option<&ConsumerNode>) {}
    fn node_removed(&mut self, _: &ConsumerNode) {}
}

fn complete(child: NodeId, requested: NodeId) -> TreeUpdate {
    let mut root = Node::new(Role::Window);
    root.set_children(vec![child]);
    TreeUpdate {
        nodes: vec![(ROOT, root), (child, Node::new(Role::TextInput))],
        tree: Some(TreeInfo::new(ROOT)),
        tree_id: TreeId::ROOT,
        focus: accesskit_focus::published_focus(requested, ROOT, [ROOT, child].into_iter()),
    }
}

#[test]
#[should_panic(expected = "is not in the node list")]
fn v103_consumer_reproduces_unpublished_focus_panic() {
    let mut tree = accesskit_consumer::Tree::new(complete(OLD, OLD), true);
    tree.update_and_process_changes(
        TreeUpdate {
            nodes: vec![],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NEW,
        },
        &mut Changes,
    );
}

#[test]
fn v103_new_focus_waits_for_tree_then_reaches_control() {
    let mut tree = accesskit_consumer::Tree::new(complete(OLD, OLD), true);
    let focus = accesskit_focus::published_focus(NEW, ROOT, [ROOT, OLD].into_iter());
    tree.update_and_process_changes(
        TreeUpdate {
            nodes: vec![],
            tree: None,
            tree_id: TreeId::ROOT,
            focus,
        },
        &mut Changes,
    );
    assert_eq!(focus, ROOT);
    let rebuilt = complete(NEW, NEW);
    assert_eq!(rebuilt.focus, NEW);
    tree.update_and_process_changes(rebuilt, &mut Changes);
}

#[test]
fn v103_dirty_update_defers_unpublished_delegate_focus() {
    let mut tree = accesskit_consumer::Tree::new(complete(OLD, OLD), true);
    let focus = accesskit_focus::published_focus(NEW, ROOT, [ROOT, OLD].into_iter());
    tree.update_and_process_changes(
        TreeUpdate {
            nodes: vec![(OLD, Node::new(Role::TextInput))],
            tree: None,
            tree_id: TreeId::ROOT,
            focus,
        },
        &mut Changes,
    );
    assert_eq!(focus, ROOT);
}

#[test]
fn v103_full_rebuild_rejects_removed_focus_and_preserves_known_focus() {
    let mut tree = accesskit_consumer::Tree::new(complete(OLD, OLD), true);
    let rebuilt = complete(NEW, OLD);
    assert_eq!(rebuilt.focus, ROOT);
    tree.update_and_process_changes(rebuilt, &mut Changes);
    assert_eq!(
        accesskit_focus::published_focus(NEW, ROOT, [ROOT, NEW].into_iter()),
        NEW
    );
    assert_eq!(
        accesskit_focus::published_focus(ROOT, ROOT, [ROOT, NEW].into_iter()),
        ROOT
    );
}

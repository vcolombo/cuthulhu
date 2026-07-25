// SPDX-License-Identifier: GPL-3.0-or-later
use document::{Delta, NodeOp, Node, ShapeKind, NodeId, IdGen, Style};
use crate::{svg_to_paths, IoError};

pub fn import_svg(
    bytes: &[u8],
    ids: &mut IdGen,
    parent: NodeId,
) -> Result<(Delta, Vec<String>), IoError> {
    let imp = svg_to_paths(bytes)?;
    let ops = imp.paths
        .into_iter()
        .map(|(path, hint)| {
            let mut node = Node::shape(ids.next(), ShapeKind::Path {
                d: path.to_svg(),
            });
            node.style = Style {
                stroke: hint.stroke,
                fill: hint.fill,
            };
            NodeOp::Add {
                parent,
                node,
                index: usize::MAX,
            }
        })
        .collect();
    Ok((Delta(ops), imp.skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_svg_produces_one_add_per_path() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#;
        let mut ids = document::IdGen::default();
        let (delta, skipped) = import_svg(svg, &mut ids, document::NodeId(1)).unwrap();
        assert_eq!(delta.0.len(), 1);
        assert!(skipped.is_empty());
    }
}

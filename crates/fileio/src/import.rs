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

    /// Import defaults to cuttable whatever the paint says — the fill-only clipart that
    /// used to import as "nothing to cut" is the case this exists for. The value is the
    /// constructor's; this pins that `import_svg` does not overwrite it from the stroke,
    /// which is the mistake the old `plan_passes` rule would invite.
    #[test]
    fn an_imported_path_is_cut_even_with_no_stroke() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"
            viewBox="0 0 10 10"><rect width="5" height="5" fill="#00ff00"/></svg>"##;
        let mut ids = document::IdGen::default();
        let (delta, _skipped) = import_svg(svg, &mut ids, document::NodeId(1)).unwrap();
        let nodes: Vec<&document::Node> = delta.0.iter()
            .filter_map(|op| match op { document::NodeOp::Add { node, .. } => Some(node), _ => None })
            .collect();
        assert!(!nodes.is_empty(), "premise: the rect imported");
        for node in nodes {
            assert_eq!(node.style.stroke, None, "premise: fill-only art has no stroke");
            assert_eq!(node.cut_line_type, document::CutLineType::Cut);
        }
    }
}

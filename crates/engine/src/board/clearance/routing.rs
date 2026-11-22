//! Append-only route fragments over a shared, indexed obstacle projection.
use super::*;

/// Seal a bounded block at a time. Completed blocks are never rebuilt during a cascade.
const BLOCK_SIZE: usize = 128;

#[derive(Default)]
pub(in crate::board) struct RouteFragments {
    objects: Vec<Object>,
    blocks: Vec<index::Index>,
    gesture_end: usize,
}
impl RouteFragments {
    /// Append one fragment under its original intent. Only each complete block is indexed;
    /// the remaining tail needs at most one small linear scan per lookup.
    pub(in crate::board) fn push(
        &mut self,
        base: &ClearanceQuery,
        contour: Contour,
        layers: Vec<Uuid>,
        source: Option<usize>,
    ) -> Result<(), CommandError> {
        base.validate_layers(&layers)?;
        let source = source.map(|i| &base.objects()[i]);
        let name = source.map_or_else(|| format!("overlay-added-{}", Uuid::from_u128(self.objects.len() as u128 + 1)), |source| source.name.clone());
        let intended_nets = source.map_or_else(Vec::new, |source| source.intended_nets.clone());
        let island = source.map_or(usize::MAX, |source| source.island);
        self.objects.push(Object { name, at: contour.core[0], contour, layers, intended_nets, island, net: None, mechanical: false });
        let len = self.objects.len();
        if len.is_multiple_of(BLOCK_SIZE) {
            self.blocks.push(index::Index::new(&self.objects[len - BLOCK_SIZE..]));
        }
        Ok(())
    }

    /// The already-fixed and proposed incoming copper share a routing guide hull.
    /// Subsequently appended displacements retain separate hulls.
    pub(in crate::board) fn finish_gesture(&mut self) {
        self.gesture_end = self.objects.len();
    }
}

/// A borrowed planning view. Base indices never move, and routing fragments follow them
/// in append order. Dropping a cascade discards only its own fragments.
#[derive(Clone, Copy)]
pub(in crate::board) struct RouteObstacles<'a> {
    base: &'a ClearanceQuery,
    fragments: Option<&'a RouteFragments>,
}
impl<'a> From<&'a ClearanceQuery> for RouteObstacles<'a> {
    /// Walkaround uses the captured board as-is, with no speculative fragments.
    fn from(base: &'a ClearanceQuery) -> Self {
        Self { base, fragments: None }
    }
}
impl<'a> RouteObstacles<'a> {
    /// Read exact base and speculative geometry through one stable index space.
    pub(in crate::board) fn new(base: &'a ClearanceQuery, fragments: &'a RouteFragments) -> Self {
        Self { base, fragments: Some(fragments) }
    }

    /// Resolve base or appended geometry without copying the source board.
    pub(in crate::board) fn get(&self, index: usize) -> Option<&'a Object> {
        let base_len = self.base.objects().len();
        if index < base_len { self.base.objects().get(index) } else { self.fragments?.objects.get(index - base_len) }
    }

    /// The shared guide never changes exact probes or fragment identities.
    pub(in crate::board) fn guide_group(&self, index: usize) -> Option<std::ops::Range<usize>> {
        let base = self.base.objects().len();
        let end = self.fragments?.gesture_end;
        (end > 1 && (base..base + end).contains(&index)).then_some(base..base + end)
    }

    /// Use the captured revision and rules for both base and fragment probes.
    pub(in crate::board) fn authority(&self) -> &Authority {
        self.base.authority()
    }

    /// Resolve the same pair requirement for original and inherited memberships.
    pub(in crate::board) fn membership_clearance(&self, left: &[usize], right: &[usize]) -> i64 {
        self.base.membership_clearance(left, right)
    }

    /// Append only hits from the packed fragment blocks and their unfinished tail.
    /// Each group returns stable identity order, just as the original index does.
    pub(in crate::board) fn candidates(&self, contour: &Contour, layers: &[Uuid]) -> Result<Vec<usize>, CommandError> {
        let mut ids = self.base.candidates(contour, layers)?;
        if let Some(fragments) = self.fragments {
            let base = self.base.objects().len();
            let bounds = contour.bounds();
            let clearance = self.authority().clearance_nm as f64;
            for (block, tree) in fragments.blocks.iter().enumerate() {
                let start = block * BLOCK_SIZE;
                ids.extend(
                    tree.candidates(bounds, clearance, layers)
                        .into_iter()
                        .map(|i| start + i)
                        .filter(|i| share_layers(layers, &fragments.objects[*i].layers))
                        .map(|i| base + i),
                );
            }
            let start = fragments.blocks.len() * BLOCK_SIZE;
            ids.extend(fragments.objects[start..].iter().enumerate().filter_map(|(offset, object)| {
                (share_layers(layers, &object.layers) && near_bounds(bounds, object.contour.bounds(), clearance)).then_some(base + start + offset)
            }));
        }
        Ok(ids)
    }

    /// Use the same membership union and exact predicates as captured board queries.
    pub(in crate::board) fn probe_membership(
        &self,
        contour: &Contour,
        layers: &[Uuid],
        nets: &[usize],
        excluded: Option<usize>,
    ) -> Result<Results, CommandError> {
        let objects = self.candidates(contour, layers)?.into_iter().filter(|i| Some(*i) != excluded).map(|i| (i, &self[i]));
        self.base.evaluate_probe(contour, nets, objects)
    }
}
impl std::ops::Index<usize> for RouteObstacles<'_> {
    type Output = Object;
    /// Resolve a candidate index returned by the same view.
    fn index(&self, index: usize) -> &Self::Output {
        let base = self.base.objects();
        if index < base.len() { &base[index] } else { &self.fragments.map_or(&[][..], |fragments| fragments.objects.as_slice())[index - base.len()] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::track::tests::{copper_layers, routed_fixture, view};

    /// Appended indexed blocks and their tail produce the same exact hits,
    /// net union and clearance decisions as a materialized projection.
    #[test]
    fn fragment_blocks_match_a_materialized_projection() {
        let session = routed_fixture();
        let layer = copper_layers(&session)[0];
        let base = session.clearance_query_with_rules(view(&session).target, &serde_json::json!({})).unwrap();
        let mut fragments = RouteFragments::default();
        let point = |x_nm, y_nm| Point { x_nm, y_nm };
        for i in 0..2 * BLOCK_SIZE + 37 {
            let x = i as i64 * 30_000;
            let contour = Contour::capsule(point(x, -20_000_000), point(x + 50_000, -19_500_000), 125_000.);
            fragments.push(&base, contour, vec![layer], (i % 2 == 0).then_some(0)).unwrap();
        }
        assert_eq!(fragments.blocks.len(), 2);
        let proposal = RouteObstacles::new(&base, &fragments);
        let full = base.with_objects(base.objects().iter().chain(&fragments.objects).cloned().collect()).unwrap();
        for x in (0..=8_000_000).step_by(75_000) {
            let test = Contour::capsule(point(x, -19_850_000), point(x, -19_700_000), 125_000.);
            assert_eq!(proposal.candidates(&test, &[layer]).unwrap(), full.candidates(&test, &[layer]).unwrap());
            assert_eq!(
                proposal.probe_membership(&test, &[layer], &[], None).unwrap().decisions,
                full.probe_membership(&test, &[layer], &[], None).unwrap().decisions
            );
        }
    }
}

//! Cached catalog discovery and query membership; painting virtualizes the resulting rows.
use super::*;
/// All native rows and searchable metadata are captured once per source/filter generation.
#[derive(Debug)]
pub(super) struct Results {
    pub all: Vec<Entry>,
    pub sources: serde_json::Value,
    search: Vec<String>,
    pub duplicates: std::collections::BTreeSet<String>,
    pub library_counts: std::collections::BTreeMap<String, usize>,
}
impl PartsBrowser {
    /// Changing criteria discards only disposable choice state, not an authored project.
    pub(crate) fn set_filter(&mut self, filter: library::filter::Filter) {
        if self.filter != filter {
            self.filter = filter;
            self.results = None;
            self.query_results = None;
            self.selected = None;
            self.part = None;
        }
    }
    /// Resolve native choices and source metadata once; idle frames share the exact result object.
    pub(super) fn catalog_results(&mut self, catalog: &library::sources::Catalog) -> Arc<Results> {
        if let Some((identity, rows)) = &self.results
            && Arc::ptr_eq(identity, &catalog.cache_identity())
        {
            return rows.clone();
        }
        let all: Vec<_> = entries("", catalog).into_iter().filter(|e| native_parts::matches_filter(e, catalog, &self.filter)).collect();
        let search = all
            .iter()
            .map(|e| {
                let mut text = format!("{} {} {} {}", e.name(), e.0.id, e.0.reference_prefix(), e.0.library);
                if let Some(part) = &e.1 {
                    text.push_str(&format!(" {} {}", part.definition.id, part.definition.library));
                    for m in catalog.manufacturers_for(&part.definition.reference()) {
                        let candidate = pcbcraft_engine::part_instances::Selection {
                            resolved: part.clone(),
                            package: Some(m.package.clone()),
                            manufacturer: Some(m.clone()),
                            requirements: Default::default(),
                        };
                        if self.filter.matches(&candidate) {
                            text.push_str(&format!(" {} {}", m.manufacturer, m.mpn));
                        }
                    }
                }
                text.to_lowercase()
            })
            .collect();
        let mut names = std::collections::BTreeMap::<String, usize>::new();
        let mut library_counts = std::collections::BTreeMap::<String, usize>::new();
        for entry in &all {
            *names.entry(entry.name().to_owned()).or_default() += 1;
            let library = entry.1.as_ref().map_or(&entry.0.library, |p| &p.definition.library);
            *library_counts.entry(library.clone()).or_default() += 1;
        }
        let duplicates = names.into_iter().filter_map(|(name, count)| (count > 1).then_some(name)).collect();
        let rows = Arc::new(Results { all, sources: catalog.inspect(), search, duplicates, library_counts });
        self.results = Some((catalog.cache_identity(), rows.clone()));
        self.query_results = None;
        rows
    }
    /// Query and collection changes rebuild membership; scrolling and hovering reuse it.
    pub(super) fn query_results(&mut self, rows: &Results, query: &str) -> Arc<Vec<Entry>> {
        if let Some((text, collection, entries)) = &self.query_results
            && text == query
            && collection == &self.collection
        {
            return entries.clone();
        }
        let matches: Arc<Vec<Entry>> = Arc::new(
            rows.all
                .iter()
                .zip(&rows.search)
                .filter(|(e, text)| e.in_collection(&self.collection) && text.contains(query))
                .map(|(e, _)| e.clone())
                .collect(),
        );
        self.query_results = Some((query.to_owned(), self.collection.clone(), matches.clone()));
        matches
    }
}

use std::collections::BTreeMap;

use agent_fw_catalog::{CatalogFacetValue, CatalogSearchFacets};

#[derive(Debug, Default)]
pub(crate) struct FacetAccumulator {
    kinds: BTreeMap<String, usize>,
    schemas: BTreeMap<String, usize>,
    tables: BTreeMap<String, usize>,
    tags: BTreeMap<String, usize>,
}

impl FacetAccumulator {
    pub fn add_kind(&mut self, value: Option<String>) {
        add_value(&mut self.kinds, value);
    }

    pub fn add_schema(&mut self, value: Option<String>) {
        add_value(&mut self.schemas, value);
    }

    pub fn add_table(&mut self, value: Option<String>) {
        add_value(&mut self.tables, value);
    }

    pub fn add_tag(&mut self, value: Option<String>) {
        add_value(&mut self.tags, value);
    }

    pub fn finish(self) -> CatalogSearchFacets {
        CatalogSearchFacets {
            kinds: finish_map(self.kinds),
            schemas: finish_map(self.schemas),
            tables: finish_map(self.tables),
            tags: finish_map(self.tags),
        }
    }
}

fn add_value(map: &mut BTreeMap<String, usize>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    *map.entry(trimmed.to_string()).or_insert(0) += 1;
}

fn finish_map(map: BTreeMap<String, usize>) -> Vec<CatalogFacetValue> {
    let mut values: Vec<CatalogFacetValue> = map
        .into_iter()
        .map(|(value, count)| CatalogFacetValue { value, count })
        .collect();
    values.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.value.cmp(&right.value))
    });
    values
}

use crate::{
    dbc::{Dbc, EmptyStringExt},
    parser::ParsedCanFrame,
    CommonConfig,
};
use auxon_sdk::api::AttrVal;

#[derive(Eq, PartialEq, Hash, Default)]
pub struct TimelineKey {
    node_name: Option<String>,
    default_name: Option<String>,
}

impl TimelineKey {
    pub fn for_parsed_frame(pcf: &ParsedCanFrame, config: &CommonConfig) -> Self {
        let mut key = TimelineKey::default();

        if config.timeline_from_node.unwrap_or(true) {
            key.node_name.clone_from(&pcf.transmitter_node);
        }
        if let Some(n) = config.default_timeline.as_ref() {
            key.default_name = Some(n.to_owned());
        }

        key
    }

    pub fn timeline_name(&self) -> &str {
        self.node_name
            .as_deref()
            .or(self.default_name.as_deref())
            .unwrap_or("canbus")
    }

    pub fn timeline_attrs(&self, dbc: &Option<Dbc>) -> Vec<(&'static str, AttrVal)> {
        let mut attrs = vec![];

        if let Some(node) = self.node_name.as_ref() {
            attrs.push(("timeline.transmitter", node.into()));
        }

        if let Some(dbc) = dbc {
            gather_dbc_attrs(dbc, &mut attrs);
        }

        attrs
    }
}

fn gather_dbc_attrs(dbc: &Dbc, attrs: &mut Vec<(&'static str, AttrVal)>) {
    if let Some(version) = (&dbc.inner.version().0).empty_opt() {
        attrs.push(("timeline.dbc.version", version.into()));
    }
    if let Some(file_name) = dbc.file_name.as_ref() {
        attrs.push(("timeline.dbc.file_name", file_name.into()));
    }
    attrs.push(("timeline.dbc.sha256", dbc.sha256.as_str().into()));
}

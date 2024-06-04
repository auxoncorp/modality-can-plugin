use crate::{convert::TimelineKey, dbc::Dbc, parser::ParsedCanFrame, HasCommonConfig};
use auxon_sdk::{
    api::{AttrKey, AttrVal, TimelineId},
    plugin_utils::ingest::Config,
};
use std::collections::HashMap;
use tracing::debug;

pub struct Sender<C: HasCommonConfig> {
    client: auxon_sdk::plugin_utils::ingest::Client,
    common_timeline_attrs: HashMap<AttrKey, AttrVal>,
    dbc: Option<Dbc>,
    config: Config<C>,
    known_timelines: HashMap<TimelineKey, TimelineId>,
    current_timeline: Option<TimelineId>,
    event_ordering: u128,
}

impl<C: HasCommonConfig> Sender<C> {
    pub fn new(
        client: auxon_sdk::plugin_utils::ingest::Client,
        common_timeline_attrs: HashMap<AttrKey, AttrVal>,
        dbc: Option<Dbc>,
        config: Config<C>,
    ) -> Self {
        Self {
            client,
            common_timeline_attrs,
            dbc,
            config,
            known_timelines: Default::default(),
            current_timeline: None,
            event_ordering: 0,
        }
    }

    pub async fn close(self) -> Result<(), anyhow::Error> {
        let mut client = self.client;
        client.flush().await?;

        if let Ok(status) = client.status().await {
            debug!(
                events_received = status.events_received,
                events_written = status.events_written,
                events_pending = status.events_pending,
                "Ingest status"
            );
        }

        Ok(())
    }

    pub async fn handle_frame(&mut self, pcf: ParsedCanFrame) -> Result<(), anyhow::Error> {
        let tl_key = TimelineKey::for_parsed_frame(&pcf, self.config.plugin.common_config());
        match self.known_timelines.get(&tl_key) {
            Some(tl_id) => {
                // It's a known timeline; switch to it if necessary
                if self.current_timeline != Some(*tl_id) {
                    self.client.switch_timeline(*tl_id).await?;
                    self.current_timeline = Some(*tl_id);
                }
            }
            None => {
                // We've never seen this timeline before; allocate an
                // id, and send its attrs.
                let tl_id = TimelineId::allocate();

                self.client.switch_timeline(tl_id).await?;
                self.current_timeline = Some(tl_id);

                let attrs: Vec<_> = self
                    .common_timeline_attrs
                    .iter()
                    .map(|(k, v)| (k.as_ref(), v.clone()))
                    .chain(tl_key.timeline_attrs(&self.dbc))
                    .collect();
                self.client
                    .send_timeline_attrs(tl_key.timeline_name(), attrs)
                    .await?;
                self.known_timelines.insert(tl_key, tl_id);
            }
        };

        let ev_name = pcf.event_name();
        let ev_attrs: Vec<_> = pcf
            .attrs
            .iter()
            .map(|(k, v)| (k.as_ref(), v.clone()))
            .collect();
        self.client
            .send_event(&ev_name, self.event_ordering, ev_attrs)
            .await?;

        self.event_ordering += 1;
        Ok(())
    }
}

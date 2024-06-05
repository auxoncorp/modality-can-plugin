# modality-can-plugin

Modality reflector plugins for CAN (Controller Area Network).

## Configuration
Lowercase names are the config keys which may be used in a reflector
config toml file. Uppercase names are environment variables which may
be used for the same configuration.

### Common
These options are used by both the collectors and the importers.

* `timeline-from-node` / `MODALITY_CAN_TIMELINE_FROM_NODE`
Should the transmitting DBC node be used as the timeline identity and name? Defaults to true.

* `default-timeline` / `MODALITY_CAN_TIMELINE`
The default timeline name used when no DBC file is provided, or there are no transmitting
nodes for a given CAN frame in the provided DBC.
Defaults to 'canbus'.

* `event-from-message` / `MODALITY_CAN_EVENT_FROM_MESSAGE`
Use the DBC message name for event naming. Defaults to true.
When no DBC file is provided, or there is no message definition, the CAN ID will be used.

* `dbc` / `MODALITY_CAN_DBC`
DBC file to use when parsing the CAN frames.

* `MODALITY_RUN_ID`
The run id to value to use in timeline metadata (`timeline.run_id`). This is used as the basis for the segmentation method used in the default Modality workspace.
Defaults to a randomly generated uuid.

* `MODALITY_AUTH_TOKEN`
The content of the auth token to use when connecting to Modality. If this is not set, the auth token used by the Modality CLI is read from `~/.config/modality_cli/.user_auth_token`

* `MODALITY_HOST`
The hostname where the modality server is running.

### SocketCAN Collector
These options are used by both the SocketCAN collector.

* `interface` / `MODALITY_CAN_INTERFACE`
The SocketCAN interface to use. Defaults to 'can0'.

* `filters`/ `MODALITY_CAN_FILTERS`
List of CAN filters to apply.
When provided via the environment variable, use the format `<id>:<mask>[:!]` for a single filter.
Can be comma-separated for multiple filters.

* `hw-timestamps` / `MODALITY_CAN_HW_TIMESTAMPS`
Enable receive hardware timestamps.
Defaults to true.

* `bitrate` / `MODALITY_CAN_BITRATE`
CAN bitrate.
Defaults to unchanged.
This is a privileged operation that requires the `CAP_NET_ADMIN` capability.

* `data-bitrate` / `MODALITY_CAN_DATA_BITRATE`
CAN FD data bitrate.
Defaults to unchanged.
This is a privileged operation that requires the `CAP_NET_ADMIN` capability.

* `restart-ms` / `MODALITY_CAN_RESTART_MS`
Set the automatic restart time (in milliseconds).
Zero means auto-restart is disabled.
Defaults to unchanged.
This is a privileged operation that requires the `CAP_NET_ADMIN` capability.

* `termination` / `MODALITY_CAN_TERMINATION`
The CAN bus termination resistance.
Defaults to unchanged.
This is a privileged operation that requires the `CAP_NET_ADMIN` capability.

* `listen-only` / `MODALITY_CAN_LISTEN_ONLY`
Set the listen-only control mode bit.
Defaults to false.
This is a privileged operation that requires the `CAP_NET_ADMIN` capability.

* `fd` / `MODALITY_CAN_FD`
Set the CAN FD control mode bit.
Defaults to false.
This is a privileged operation that requires the `CAP_NET_ADMIN` capability.

## Adapter Concept Mapping
The following describes the default mapping between CAN/DBC concepts and Modality's concepts.

* Timeline creation is customizable, based on the `timeline-from-node` configuration option.
  - If a DBC file is given, then the transmitting node for a given message will be used for
    the timeline.
  - If the message is not known to the DBC, or a DBC was not given, then the `default-timeline`
    name is used.

* Events are named based on the message name in the DBC, or the CAN ID if not found or no
  DBC file was provided.

* DBC message signals are parsed into attribute key/value pairs.

* CAN frame-level details (e.g. DLC) are logged with the prefix `event.frame.`.

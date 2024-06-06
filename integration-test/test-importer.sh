#!/usr/bin/env bash
set -e

/modality-reflector import --ingest-protocol-parent-url ${INGEST_PROTOCOL_PARENT_URL} can /candump.log
/modality workspace sync-indices
/conform spec eval --file /importer.speqtr --dry-run

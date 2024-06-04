#!/usr/bin/env bash
set -e

/publish-test-data.py
/modality wait-until '6 @ canbus aggregate count() > 0'
/modality workspace sync-indices
/conform spec eval --file /collector.speqtr --dry-run

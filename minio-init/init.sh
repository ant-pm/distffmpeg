#!/bin/sh
set -e

mc alias set local http://minio:9000 minioadmin minioadmin
mc mb --ignore-existing local/uploads
mc mb --ignore-existing local/outputs
echo "MinIO init complete."

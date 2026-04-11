ORG    = ghcr.io/ant-pm
TAG    = latest

BACKEND_IMAGE   = $(ORG)/ffmpeg-backend
DASHBOARD_IMAGE = $(ORG)/ffmpeg-dashboard
WORKER_IMAGE    = $(ORG)/ffmpeg-worker

.PHONY: build backend dashboard worker

build: backend dashboard worker

backend:
	docker buildx build \
		--platform linux/amd64,linux/arm64 \
		-t $(BACKEND_IMAGE):$(TAG) \
		--push \
		backend/

dashboard:
	docker buildx build \
		--platform linux/amd64,linux/arm64 \
		-t $(DASHBOARD_IMAGE):$(TAG) \
		--push \
		dashboard/

worker:
	docker buildx build \
		--platform linux/amd64,linux/arm64 \
		-t $(WORKER_IMAGE):$(TAG) \
		--push \
		worker/

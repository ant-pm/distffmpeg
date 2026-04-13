<template>
  <div>
    <h2 class="mb-3 text-lg font-semibold tracking-tight">Jobs</h2>

    <p v-if="jobs.length === 0" class="text-sm text-muted-foreground">
      No jobs yet. Upload a file to get started.
    </p>

    <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
      <div
        v-for="job in jobs"
        :key="job.id"
        class="rounded-lg border bg-card p-4 transition-colors"
        :class="job.status === 'processing' ? 'border-amber-500/60' : 'border-border'"
      >
        <!-- Header -->
        <div class="flex items-start justify-between gap-2">
          <span class="truncate text-sm font-semibold text-foreground">{{ job.filename }}</span>
          <div class="flex shrink-0 items-center gap-2">
            <span class="text-xs text-muted-foreground">{{ timeAgo(job.created_at) }}</span>
            <Badge :variant="statusVariant(job.status)">{{ formatStatus(job.status) }}</Badge>
          </div>
        </div>

        <!-- Codec / format row -->
        <div class="mt-3 space-y-2">
          <div class="flex items-center justify-between">
            <span class="text-xs text-muted-foreground">Output</span>
            <span class="font-mono text-xs text-foreground">{{ job.output_format.toUpperCase() }}</span>
          </div>

          <div class="flex items-center justify-between">
            <span class="text-xs text-muted-foreground">Video</span>
            <span class="font-mono text-xs text-foreground">{{ formatCodec(job.encoding.video_codec) }}</span>
          </div>

          <div class="flex items-center justify-between">
            <span class="text-xs text-muted-foreground">Audio</span>
            <span class="font-mono text-xs text-foreground">{{ formatCodec(job.encoding.audio_codec) }}</span>
          </div>

          <div class="flex items-center justify-between">
            <span class="text-xs text-muted-foreground">CRF / Preset</span>
            <span class="font-mono text-xs text-foreground">{{ job.encoding.crf }} · {{ job.encoding.preset }}</span>
          </div>

          <div v-if="job.encoding.resolution" class="flex items-center justify-between">
            <span class="text-xs text-muted-foreground">Resolution</span>
            <span class="font-mono text-xs text-foreground">{{ job.encoding.resolution.replace(':', '×') }}</span>
          </div>

          <!-- Timing -->
          <div v-if="job.started_at" class="flex items-center justify-between">
            <span class="text-xs text-muted-foreground">Time</span>
            <span class="font-mono text-xs" :class="job.status === 'processing' ? 'text-amber-400' : 'text-foreground'">
              {{ elapsed(job) }}
            </span>
          </div>
        </div>

        <!-- Progress bar -->
        <div v-if="job.status === 'processing'" class="mt-3">
          <div class="mb-1 flex items-center justify-between">
            <span class="text-xs text-muted-foreground">
              {{ job.chunks_completed === job.chunk_count ? 'Merging' : 'Encoding' }}
            </span>
            <span class="font-mono text-xs text-amber-400">
              <template v-if="job.chunk_count">
                {{ job.chunks_completed }} / {{ job.chunk_count }} chunks
              </template>
              <template v-else>—</template>
            </span>
          </div>
          <div class="h-1.5 w-full overflow-hidden rounded-full bg-white/10">
            <div
              class="h-full rounded-full bg-amber-400 transition-all duration-300"
              :style="{ width: `${(job.progress ?? 0) * 100}%` }"
            />
          </div>
        </div>

        <!-- Chunk grid -->
        <div v-if="job.chunks && job.chunks.length > 0" class="mt-3">
          <div class="mb-1.5 text-xs text-muted-foreground">Chunks</div>
          <div class="flex flex-wrap gap-1">
            <div
              v-for="chunk in job.chunks"
              :key="chunk.index"
              class="h-4 w-4 rounded-sm transition-colors"
              :class="chunkColor(chunk.status)"
              :title="chunkTitle(chunk)"
            />
          </div>

          <!-- Worker summary -->
          <div v-if="workerSummary(job).length > 0" class="mt-2 space-y-0.5">
            <div
              v-for="w in workerSummary(job)"
              :key="w.name"
              class="flex items-center justify-between"
            >
              <span class="truncate font-mono text-xs text-muted-foreground">{{ w.name }}</span>
              <span class="ml-2 shrink-0 font-mono text-xs text-foreground">{{ w.count }} chunk{{ w.count !== 1 ? 's' : '' }}</span>
            </div>
          </div>
        </div>

        <!-- Error -->
        <p v-if="job.error" class="mt-2 text-xs text-red-400">{{ job.error }}</p>

        <!-- Download -->
        <div v-if="job.status === 'completed'" class="mt-3">
          <Button variant="outline" size="sm" class="w-full text-xs" @click="download(job)">
            <Download class="mr-1.5 h-3 w-3" />
            Download
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Download } from 'lucide-vue-next'

defineProps({
  jobs: { type: Array, required: true },
})

const now = ref(Date.now())
let timer = null
onMounted(() => { timer = setInterval(() => { now.value = Date.now() }, 1000) })
onUnmounted(() => clearInterval(timer))

function statusVariant(s) {
  return { pending_upload: 'secondary', queued: 'info', processing: 'warning', completed: 'success', failed: 'destructive' }[s] ?? 'secondary'
}

function formatStatus(s) {
  return { pending_upload: 'Pending', queued: 'Queued', processing: 'Processing', completed: 'Done', failed: 'Failed' }[s] ?? s
}

function formatCodec(c) {
  return { h264: 'H.264', h265: 'H.265 / HEVC', av1: 'AV1 (SVT)', vp9: 'VP9', copy: 'copy', aac: 'AAC', opus: 'Opus', flac: 'FLAC', mp3: 'MP3' }[c] ?? c
}

function timeAgo(iso) {
  const secs = Math.floor((Date.now() - new Date(iso)) / 1000)
  if (secs < 60) return `${secs}s ago`
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`
  return `${Math.floor(secs / 3600)}h ago`
}

function elapsed(job) {
  if (!job.started_at) return null
  const end = job.finished_at ? new Date(job.finished_at).getTime() : now.value
  const secs = Math.max(0, Math.floor((end - new Date(job.started_at).getTime()) / 1000))
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return m > 0 ? `${m}m ${s}s` : `${s}s`
}

function chunkColor(status) {
  return {
    queued:  'bg-white/15',
    running: 'bg-amber-400 animate-pulse',
    done:    'bg-green-500',
    failed:  'bg-red-500',
  }[status] ?? 'bg-white/10'
}

function chunkTitle(chunk) {
  const base = `Chunk ${chunk.index} — ${chunk.status}`
  return chunk.worker ? `${base}\n${chunk.worker}` : base
}

function workerSummary(job) {
  const counts = {}
  for (const chunk of job.chunks ?? []) {
    if (chunk.worker) {
      counts[chunk.worker] = (counts[chunk.worker] ?? 0) + 1
    }
  }
  return Object.entries(counts)
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => b.count - a.count)
}

async function download(job) {
  try {
    const res = await fetch(`/api/jobs/${job.id}/download`)
    if (!res.ok) throw new Error('Failed to get download URL')
    const { download_url } = await res.json()
    window.open(download_url, '_blank')
  } catch (e) {
    alert(e.message)
  }
}
</script>

<template>
  <div class="min-h-screen bg-background text-foreground">
    <div class="mx-auto max-w-5xl px-6 py-8">

      <header class="mb-8 flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-bold tracking-tight">FFmpeg Cluster</h1>
          <p class="mt-0.5 text-sm text-muted-foreground">{{ workerCount }} worker{{ workerCount !== 1 ? 's' : '' }} online</p>
        </div>
        <Badge :variant="wsConnected ? 'success' : 'destructive'">
          {{ wsConnected ? 'Live' : 'Offline' }}
        </Badge>
      </header>

      <div class="space-y-8">
        <UploadForm @job-created="onJobCreated" />
        <JobList :jobs="jobs" />
      </div>

    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { Badge } from '@/components/ui/badge'
import UploadForm from './components/UploadForm.vue'
import JobList from './components/JobList.vue'

const jobs = ref([])
const wsConnected = ref(false)
let ws = null
let reconnectTimer = null

const workerCount = computed(() => {
  // Count jobs that are currently processing as a proxy for active workers
  return wsConnected.value ? 1 : 0
})

function connectWs() {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const host = window.location.host
  ws = new WebSocket(`${protocol}//${host}/ws/client`)

  ws.onopen = () => { wsConnected.value = true }
  ws.onclose = () => {
    wsConnected.value = false
    reconnectTimer = setTimeout(connectWs, 2000)
  }
  ws.onerror = () => ws.close()

  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data)
    if (msg.type === 'jobs_list') {
      jobs.value = msg.jobs
    } else if (msg.type === 'job_update') {
      const idx = jobs.value.findIndex(j => j.id === msg.job.id)
      if (idx >= 0) jobs.value[idx] = msg.job
      else jobs.value.unshift(msg.job)
    }
  }
}

function onJobCreated(job) {
  if (!jobs.value.find(j => j.id === job.id)) {
    jobs.value.unshift(job)
  }
}

onMounted(() => connectWs())
onUnmounted(() => {
  clearTimeout(reconnectTimer)
  if (ws) ws.close()
})
</script>

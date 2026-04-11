<template>
  <div>
    <h2 class="mb-3 text-lg font-semibold tracking-tight">New Job</h2>
    <div class="rounded-lg border border-border bg-card p-5">
      <form @submit.prevent="handleSubmit" class="space-y-4">

        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <div class="sm:col-span-2 space-y-1.5">
            <Label class="text-xs text-muted-foreground uppercase tracking-wider">File</Label>
            <input
              type="file"
              ref="fileInput"
              required
              accept="video/*,audio/*"
              class="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1.5 text-sm text-foreground file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-muted-foreground cursor-pointer"
            />
          </div>

          <div class="space-y-1.5">
            <Label class="text-xs text-muted-foreground uppercase tracking-wider">Container</Label>
            <Select v-model="outputFormat">
              <SelectTrigger class="h-9 text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="mp4">MP4</SelectItem>
                <SelectItem value="mkv">MKV</SelectItem>
                <SelectItem value="webm">WebM</SelectItem>
                <SelectItem value="mov">MOV</SelectItem>
                <SelectItem value="mp3">MP3</SelectItem>
                <SelectItem value="flac">FLAC</SelectItem>
                <SelectItem value="ogg">OGG</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div class="space-y-1.5">
            <Label class="text-xs text-muted-foreground uppercase tracking-wider">Video Codec</Label>
            <Select v-model="videoCodec">
              <SelectTrigger class="h-9 text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="h264">H.264 (x264)</SelectItem>
                <SelectItem value="h265">H.265 / HEVC</SelectItem>
                <SelectItem value="av1">AV1 (SVT-AV1)</SelectItem>
                <SelectItem value="vp9">VP9</SelectItem>
                <SelectItem value="copy">Copy stream</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div class="space-y-1.5">
            <Label class="text-xs text-muted-foreground uppercase tracking-wider">Audio Codec</Label>
            <Select v-model="audioCodec">
              <SelectTrigger class="h-9 text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="aac">AAC</SelectItem>
                <SelectItem value="opus">Opus</SelectItem>
                <SelectItem value="mp3">MP3</SelectItem>
                <SelectItem value="flac">FLAC</SelectItem>
                <SelectItem value="copy">Copy stream</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div class="space-y-1.5">
            <Label class="text-xs text-muted-foreground uppercase tracking-wider">Preset</Label>
            <Select v-model="preset">
              <SelectTrigger class="h-9 text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="ultrafast">Ultrafast</SelectItem>
                <SelectItem value="fast">Fast</SelectItem>
                <SelectItem value="medium">Medium</SelectItem>
                <SelectItem value="slow">Slow</SelectItem>
                <SelectItem value="veryslow">Very Slow</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <Label class="text-xs text-muted-foreground uppercase tracking-wider">CRF — Quality</Label>
            <span class="font-mono text-sm text-foreground">{{ crf }}</span>
          </div>
          <Slider v-model="crf" :min="0" :max="51" :step="1" />
          <p class="text-xs text-muted-foreground">Lower = better quality &amp; larger file. Typical: 18–28.</p>
        </div>

        <div class="space-y-1.5">
          <Label class="text-xs text-muted-foreground uppercase tracking-wider">Scale Resolution</Label>
          <Select v-model="resolution">
            <SelectTrigger class="h-9 text-sm">
              <SelectValue placeholder="Keep original" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="original">Keep original</SelectItem>
              <SelectItem value="3840:2160">4K — 3840×2160</SelectItem>
              <SelectItem value="1920:1080">1080p — 1920×1080</SelectItem>
              <SelectItem value="1280:720">720p — 1280×720</SelectItem>
              <SelectItem value="854:480">480p — 854×480</SelectItem>
              <SelectItem value="640:360">360p — 640×360</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <Button type="submit" :disabled="uploading" class="w-full">
          <template v-if="uploading">
            <Loader2 class="mr-2 h-4 w-4 animate-spin" />
            Uploading {{ uploadProgress }}%
          </template>
          <template v-else>
            <Upload class="mr-2 h-4 w-4" />
            Upload &amp; Transcode
          </template>
        </Button>

        <p v-if="error" class="text-xs text-red-400">{{ error }}</p>
      </form>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Select, SelectTrigger, SelectContent, SelectItem, SelectValue } from '@/components/ui/select'
import { Slider } from '@/components/ui/slider'
import { Upload, Loader2 } from 'lucide-vue-next'

const emit = defineEmits(['job-created'])

const fileInput = ref(null)
const outputFormat = ref('mp4')
const videoCodec = ref('h265')
const audioCodec = ref('aac')
const preset = ref('medium')
const crf = ref(23)
const resolution = ref('original')
const uploading = ref(false)
const uploadProgress = ref(0)
const error = ref('')

async function handleSubmit() {
  const file = fileInput.value?.files?.[0]
  if (!file) return

  uploading.value = true
  uploadProgress.value = 0
  error.value = ''

  try {
    const res = await fetch('/api/jobs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        filename: file.name,
        output_format: outputFormat.value,
        encoding: {
          video_codec: videoCodec.value,
          audio_codec: audioCodec.value,
          preset: preset.value,
          crf: crf.value,
          resolution: resolution.value === 'original' ? null : resolution.value,
        },
      }),
    })

    if (!res.ok) throw new Error('Failed to create job')
    const { job, upload_url } = await res.json()
    emit('job-created', job)

    await new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest()
      xhr.open('PUT', upload_url)
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable) uploadProgress.value = Math.round((e.loaded / e.total) * 100)
      }
      xhr.onload = () => xhr.status < 300 ? resolve() : reject(new Error(`Upload failed: HTTP ${xhr.status}`))
      xhr.onerror = () => reject(new Error('Upload failed'))
      xhr.send(file)
    })

    const markRes = await fetch(`/api/jobs/${job.id}/uploaded`, { method: 'POST' })
    if (!markRes.ok) throw new Error('Failed to start transcoding')
    fileInput.value.value = ''
  } catch (e) {
    error.value = e.message
  } finally {
    uploading.value = false
  }
}
</script>

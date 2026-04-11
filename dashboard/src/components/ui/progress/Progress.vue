<template>
  <div :class="cn('relative h-4 w-full overflow-hidden rounded-full bg-secondary', $attrs.class)">
    <div
      class="h-full bg-primary transition-all duration-300"
      :style="{ width: `${modelValue}%` }"
    />
  </div>
</template>

<script setup>
import { cn } from '@/lib/utils'

defineProps({
  modelValue: { type: Number, default: 0 },
})
</script>

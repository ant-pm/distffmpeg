<template>
  <div :class="cn(badgeVariants({ variant }), $attrs.class)">
    <slot />
  </div>
</template>

<script setup>
import { cva } from 'class-variance-authority'
import { cn } from '@/lib/utils'

const badgeVariants = cva(
  'inline-flex items-center rounded px-2 py-0.5 text-xs font-medium border',
  {
    variants: {
      variant: {
        default: 'border-transparent bg-white/10 text-white',
        secondary: 'border-white/10 text-white/50',
        destructive: 'border-red-500/30 bg-red-500/10 text-red-400',
        outline: 'border-white/20 text-white/70',
        success: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-400',
        warning: 'border-amber-500/30 bg-amber-500/10 text-amber-400',
        info: 'border-blue-500/30 bg-blue-500/10 text-blue-400',
      },
    },
    defaultVariants: { variant: 'default' },
  }
)

defineProps({
  variant: { type: String, default: 'default' },
})
</script>

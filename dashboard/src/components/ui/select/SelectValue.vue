<template>
  <SelectValue v-bind="$attrs">
    <slot />
  </SelectValue>
</template>

<script setup>
import { SelectValue } from 'radix-vue'
</script>

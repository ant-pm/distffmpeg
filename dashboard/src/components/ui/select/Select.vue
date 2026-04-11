<template>
  <SelectRoot v-bind="$attrs">
    <slot />
  </SelectRoot>
</template>

<script setup>
import { SelectRoot } from 'radix-vue'
</script>

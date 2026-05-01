<script setup lang="ts">
  import { TooltipProvider } from '~/components/ui/tooltip'
  import { NotificationToast } from '~/components/ui/notification-toast'

  const { isDark } = useTheme()
  const { showUpdateDialog, showAboutDialog, showSettingsDialog, initializeMenu } = useAppMenu()

  useHead({
    title: 'BoraBR',
    meta: [
      { name: 'description', content: 'BoraBR is an AI-native issue tracker built for local workflows' },
      { name: 'theme-color', content: () => isDark.value ? '#1e1e1e' : '#ffffff' },
    ],
    htmlAttrs: {
      lang: 'en',
      class: () => isDark.value ? 'dark' : '',
    },
  })

  onMounted(() => {
    initializeMenu()
  })
</script>

<template>
  <TooltipProvider>
    <NuxtPage />
    <LayoutUpdateDialog v-model:open="showUpdateDialog" />
    <LayoutAboutDialog v-model:open="showAboutDialog" />
    <LayoutSettingsDialog v-model:open="showSettingsDialog" />
    <NotificationToast />
  </TooltipProvider>
</template>

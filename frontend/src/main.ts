import { createApp } from 'vue'

import '@fontsource/ibm-plex-sans/latin-400.css'
import '@fontsource/ibm-plex-sans/latin-500.css'
import '@fontsource/ibm-plex-sans/latin-600.css'
import '@fontsource/ibm-plex-sans-condensed/latin-500.css'
import '@fontsource/ibm-plex-sans-condensed/latin-600.css'
import '@fontsource/ibm-plex-mono/latin-400.css'

import './styles/tokens.css'
import './styles/base.css'
import './styles/tags.css'

import App from './App.vue'
import { router } from './router'

createApp(App).use(router).mount('#app')

<template>
  <div class="vp-page">
    <div class="vp-bar">
      <button class="vp-btn" title="返回" @click="$emit('close')">
        <Icon name="chevron-left" :size="20" />
      </button>
      <span class="vp-title" :title="file.name">{{ file.name }}</span>
      <div class="vp-actions">
        <div class="vp-ext-wrap">
          <button class="vp-btn" title="用外部播放器打开" @click="menuOpen = !menuOpen">
            <Icon name="monitor" :size="18" />
          </button>
          <div v-if="menuOpen" class="vp-ext-mask" @click="menuOpen = false"></div>
          <div v-if="menuOpen" class="vp-ext-menu">
            <div class="vp-ext-title">外部播放器</div>
            <button v-for="p in PLAYERS" :key="p.name" class="vp-ext-item" @click="openExternal(p)">
              <span class="vp-ext-name">{{ p.name }}</span>
              <span class="vp-ext-desc">{{ p.desc }}</span>
            </button>
            <button class="vp-ext-item" @click="copyUrl">
              <span class="vp-ext-name">{{ copied ? '已复制 ✓' : '复制播放链接' }}</span>
              <span class="vp-ext-desc">手动粘贴到播放器</span>
            </button>
          </div>
        </div>
      </div>
    </div>

    <div class="vp-stage">
      <video :src="src" controls autoplay class="vp-video"></video>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import Icon from './Icon.vue'

const props = defineProps({
  file: { type: Object, required: true },
  src: { type: String, required: true }
})
defineEmits(['close'])

const menuOpen = ref(false)
const copied = ref(false)

// 外部播放器 URL Scheme（与 OpenList 前端一致的方式）
const PLAYERS = [
  { name: 'IINA', desc: 'macOS', url: (u) => `iina://weblink?url=${encodeURIComponent(u)}` },
  { name: 'PotPlayer', desc: 'Windows', url: (u) => `potplayer://${u}` },
  { name: 'VLC', desc: '全平台', url: (u) => `vlc://${u}` },
  { name: 'nPlayer', desc: 'iOS / Android', url: (u) => `nplayer-${u}` },
  { name: 'Infuse', desc: 'iOS / tvOS', url: (u) => `infuse://x-callback-url/play?url=${encodeURIComponent(u)}` },
  {
    name: 'MX Player',
    desc: 'Android',
    url: (u) => `intent:${encodeURIComponent(u)}#Intent;package=com.mxtech.videoplayer.ad;end`
  }
]

// 播放链接需要绝对地址，外部播放器才能访问
const absoluteUrl = computed(() => {
  try {
    return new URL(props.src, window.location.href).href
  } catch {
    return props.src
  }
})

function openExternal(p) {
  menuOpen.value = false
  const schemeUrl = p.url(absoluteUrl.value)
  // 用 location.href 触发协议唤起，失败时浏览器不跳转也不会丢失当前页面
  window.location.href = schemeUrl
}

async function copyUrl() {
  try {
    await navigator.clipboard.writeText(absoluteUrl.value)
  } catch {
    const ta = document.createElement('textarea')
    ta.value = absoluteUrl.value
    document.body.appendChild(ta)
    ta.select()
    document.execCommand('copy')
    ta.remove()
  }
  copied.value = true
  setTimeout(() => {
    copied.value = false
    menuOpen.value = false
  }, 900)
}
</script>

<style scoped>
.vp-page {
  position: fixed;
  inset: 0;
  z-index: 150;
  display: flex;
  flex-direction: column;
  background: #0b0c10;
}
.vp-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 52px;
  padding: 0 12px;
  background: #12141a;
  border-bottom: 1px solid #23252d;
  flex-shrink: 0;
}
.vp-btn {
  width: 36px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: 8px;
  color: #c9cbd4;
  cursor: pointer;
  flex-shrink: 0;
}
.vp-btn:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
}
.vp-title {
  flex: 1;
  min-width: 0;
  font-size: 13.5px;
  color: #e6e7ec;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.vp-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.vp-stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 0;
  background: #000;
}
.vp-video {
  display: block;
  width: 100%;
  height: 100%;
  outline: none;
}

/* 外部播放器菜单 */
.vp-ext-wrap {
  position: relative;
}
.vp-ext-mask {
  position: fixed;
  inset: 0;
  z-index: 1;
}
.vp-ext-menu {
  position: absolute;
  top: 44px;
  right: 0;
  z-index: 2;
  min-width: 210px;
  padding: 6px;
  background: #1a1c23;
  border: 1px solid #2b2e37;
  border-radius: 10px;
  box-shadow: var(--ol-shadow-lg);
}
.vp-ext-title {
  padding: 6px 10px 8px;
  font-size: 11.5px;
  color: #8a8d99;
}
.vp-ext-item {
  display: flex;
  flex-direction: column;
  gap: 1px;
  width: 100%;
  padding: 8px 10px;
  background: transparent;
  border: none;
  border-radius: 7px;
  text-align: left;
  cursor: pointer;
}
.vp-ext-item:hover {
  background: rgba(255, 255, 255, 0.08);
}
.vp-ext-name {
  font-size: 13px;
  color: #e6e7ec;
}
.vp-ext-desc {
  font-size: 11px;
  color: #8a8d99;
}
</style>

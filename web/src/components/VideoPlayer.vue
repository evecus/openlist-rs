<template>
  <div class="player-mask" @click.self="$emit('close')">
    <div class="player-box">
      <div class="player-bar">
        <span class="player-title">{{ file.name }}</span>
        <button class="btn-icon btn-ghost" @click="$emit('close')">
          <Icon name="close" :size="18" />
        </button>
      </div>
      <video :src="src" controls autoplay class="player-video"></video>
    </div>
  </div>
</template>

<script setup>
import Icon from './Icon.vue'

defineProps({
  file: { type: Object, required: true },
  src: { type: String, required: true }
})
defineEmits(['close'])
</script>

<style scoped>
.player-mask {
  position: fixed;
  inset: 0;
  z-index: 150;
  background: rgba(10, 12, 20, 0.86);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}
.player-box {
  width: min(94vw, 1120px);
  background: #101114;
  border-radius: 12px;
  overflow: hidden;
  box-shadow: var(--ol-shadow-lg);
}
.player-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 16px;
  background: #16181d;
  border-bottom: 1px solid #24262c;
}
.player-title {
  font-size: 13px;
  color: #c9cbd4;
  word-break: break-all;
  padding-right: 12px;
}
.player-video {
  display: block;
  width: 100%;
  max-height: 80vh;
  background: #000;
}
.btn-ghost {
  color: #c9cbd4;
}
.btn-ghost:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
}
</style>

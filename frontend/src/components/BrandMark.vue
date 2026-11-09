<template>
  <span class="brand">
    <span class="mark checker" aria-hidden="true"><i></i><i></i></span>
    <span class="word">git-opt</span>
  </span>
</template>

<style scoped>
.brand {
  display: inline-flex;
  align-items: center;
  gap: 0.55rem;
}
/* A 2×2 pixel tile: two solid pixels over the transparency grid. */
.mark {
  position: relative;
  width: 1.25rem;
  height: 1.25rem;
  border-radius: 3px;
  background-size: 10px 10px;
  overflow: hidden;
  flex: none;
}
.mark i {
  position: absolute;
  width: 50%;
  height: 50%;
  background: var(--accent);
}
.mark i:first-child {
  top: 0;
  left: 0;
}
.mark i:last-child {
  bottom: 0;
  right: 0;
}
.word {
  font-family: var(--font-display);
  font-weight: 600;
  font-size: var(--fs-lg);
  letter-spacing: -0.01em;
}
</style>

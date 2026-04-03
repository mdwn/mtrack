<!-- *     * Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
     *
     * This program is free software: you can redistribute it and/or modify it under
     * the terms of the GNU General Public License as published by the Free Software
     * Foundation, version 3.
     *
     * This program is distributed in the hope that it will be useful, but WITHOUT
     * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
     * FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
     *
     * You should have received a copy of the GNU General Public License along with
     * this program. If not, see <https://www.gnu.org/licenses/>.
     *
     * -->
<script lang="ts">
  interface Props {
    peaks: number[];
    height?: number;
    color?: string;
  }

  let {
    peaks,
    height = 32,
    color = "rgba(94, 202, 234, 0.4)",
  }: Props = $props();
  let canvas: HTMLCanvasElement | undefined = $state();

  $effect(() => {
    if (!canvas) return;
    const p = peaks;
    const c = color;
    const h = height;

    const id = requestAnimationFrame(() => {
      if (!canvas) return;
      const w = canvas.clientWidth;
      if (w === 0) return;

      const dpr = window.devicePixelRatio || 1;
      const scaledW = Math.round(w * dpr);
      const scaledH = Math.round(h * dpr);

      if (canvas.width !== scaledW || canvas.height !== scaledH) {
        canvas.width = scaledW;
        canvas.height = scaledH;
      }

      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      if (p.length > 0) {
        const barWidth = w / p.length;
        ctx.fillStyle = c;
        for (let i = 0; i < p.length; i++) {
          const barHeight = p[i] * h;
          const x = i * barWidth;
          const y = (h - barHeight) / 2;
          ctx.fillRect(x, y, Math.max(barWidth - 0.5, 1), barHeight);
        }
      }
    });

    return () => cancelAnimationFrame(id);
  });
</script>

<canvas bind:this={canvas} class="waveform" style:height="{height}px"></canvas>

<style>
  .waveform {
    width: 100%;
    display: block;
  }
</style>

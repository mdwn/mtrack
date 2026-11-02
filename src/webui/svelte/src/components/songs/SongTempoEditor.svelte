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
  import type { TempoConfig } from "../../lib/api/songs";
  import {
    configToSection,
    sectionToConfigSnapped,
  } from "../../lib/tempoConvert";
  import TempoEditor from "../lighting/TempoEditor.svelte";

  interface Props {
    /** The song.yaml `tempo:` block, or null when not configured. */
    tempo: TempoConfig | null;
    onchange: (tempo: TempoConfig | null) => void;
    songName: string;
    hasBeatGrid?: boolean;
    hasMidi?: boolean;
  }

  let {
    tempo,
    onchange,
    songName,
    hasBeatGrid = false,
    hasMidi = false,
  }: Props = $props();

  let section = $derived(tempo ? configToSection(tempo) : undefined);
</script>

<TempoEditor
  tempo={section}
  {songName}
  {hasBeatGrid}
  {hasMidi}
  measureOnly
  onchange={(updated) =>
    // The editor is measure-only, so nothing is snapped or dropped; the
    // changes come back ordered, as the song format needs them.
    onchange(updated ? sectionToConfigSnapped(updated, null).config : null)}
/>

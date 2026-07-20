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
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { uploadTrack, type SongFile } from "../../lib/api/songs";
  import FilePicker from "./FilePicker.svelte";
  import FileUpload from "./FileUpload.svelte";

  /** An audio file reference in a song config, with the same affordances as
   * the samples editor: manual path, pick an existing song file, browse the
   * filesystem (parent-owned dialog), or drop a file to upload it into the
   * song directory. */
  interface Props {
    value: string;
    placeholder?: string;
    songName: string;
    /** The song directory listing, for the pick-existing shortcut. */
    files?: SongFile[];
    onchange: (path: string | undefined) => void;
    /** When present, shows the filesystem-browse button. */
    onbrowse?: () => void;
  }

  let {
    value,
    placeholder = "",
    songName,
    files = [],
    onchange,
    onbrowse,
  }: Props = $props();

  let uploading = $state(false);
  let uploadMsg = $state("");

  async function handleUpload(dropped: File[]) {
    if (dropped.length === 0) return;
    uploading = true;
    uploadMsg = "";
    try {
      const res = await uploadTrack(songName, dropped[0]);
      if (!res.ok) {
        throw new Error(`Upload failed (${res.status})`);
      }
      const data = await res.json();
      onchange((data.file as string) ?? dropped[0].name);
      uploadMsg = get(t)("songFile.uploaded", {
        values: { name: dropped[0].name },
      });
      setTimeout(() => (uploadMsg = ""), 3000);
    } catch (e) {
      uploadMsg = e instanceof Error ? e.message : String(e);
    } finally {
      uploading = false;
    }
  }
</script>

<div class="song-file-field">
  <input
    type="text"
    class="input"
    {placeholder}
    {value}
    onchange={(e) => {
      const v = (e.target as HTMLInputElement).value.trim();
      onchange(v || undefined);
    }}
  />
  <FilePicker
    {files}
    fileType="audio"
    label={$t("songFile.useExisting")}
    onpick={(filename) => onchange(filename)}
  />
  <div class="file-actions">
    {#if onbrowse}
      <button type="button" class="btn btn-sm" onclick={onbrowse}
        >{$t("samples.browseFilesystem")}</button
      >
    {/if}
  </div>
  <FileUpload
    accept=".wav,.flac,.mp3,.ogg,.aac,.m4a,.mp4,.aiff,.aif"
    label={uploading ? $t("common.uploading") : $t("samples.dropAudio")}
    onupload={handleUpload}
  />
  {#if uploadMsg}
    <div class="upload-msg">{uploadMsg}</div>
  {/if}
</div>

<style>
  .song-file-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .song-file-field .input {
    min-height: 44px;
  }
  .file-actions {
    display: flex;
    gap: 6px;
  }
  .file-actions:empty {
    display: none;
  }
  .upload-msg {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>

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
    tags: string[];
    onchange: (tags: string[]) => void;
    placeholder?: string;
  }

  let { tags, onchange, placeholder = "Add tag..." }: Props = $props();

  let inputValue = $state("");
  let inputEl: HTMLInputElement | undefined = $state();

  function addTag(raw: string) {
    const tag = raw
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9_-]/g, "");
    if (!tag || tags.includes(tag)) {
      inputValue = "";
      return;
    }
    onchange([...tags, tag]);
    inputValue = "";
  }

  function removeTag(index: number) {
    onchange(tags.filter((_, i) => i !== index));
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      addTag(inputValue);
    } else if (e.key === "Backspace" && inputValue === "" && tags.length > 0) {
      removeTag(tags.length - 1);
    }
  }

  function handleBlur() {
    if (inputValue.trim()) {
      addTag(inputValue);
    }
  }

  function handlePaste(e: ClipboardEvent) {
    e.preventDefault();
    const text = e.clipboardData?.getData("text") ?? "";
    const newTags = text
      .split(/[,\s]+/)
      .map((t) =>
        t
          .trim()
          .toLowerCase()
          .replace(/[^a-z0-9_-]/g, ""),
      )
      .filter((t) => t && !tags.includes(t));
    if (newTags.length > 0) {
      onchange([...tags, ...newTags]);
    }
    inputValue = "";
  }
</script>

<div
  class="tag-input-wrap"
  role="textbox"
  tabindex="-1"
  onclick={() => inputEl?.focus()}
  onkeydown={() => {}}
>
  {#each tags as tag, i (tag + i)}
    <span class="tag-chip">
      {tag}
      <button
        class="tag-remove"
        type="button"
        onclick={(e) => {
          e.stopPropagation();
          removeTag(i);
        }}
        aria-label={`Remove ${tag}`}>&times;</button
      >
    </span>
  {/each}
  <input
    bind:this={inputEl}
    class="tag-text-input"
    type="text"
    bind:value={inputValue}
    {placeholder}
    onkeydown={handleKeydown}
    onblur={handleBlur}
    onpaste={handlePaste}
  />
</div>

<style>
  .tag-input-wrap {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 4px 8px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: text;
    min-height: 32px;
    align-items: center;
  }
  .tag-input-wrap:focus-within {
    border-color: var(--border-focus);
  }
  .tag-chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px 6px;
    background: var(--bg-card-hover);
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 13px;
    color: var(--text);
    white-space: nowrap;
  }
  .tag-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    padding: 0;
    margin-left: 2px;
    background: none;
    border: none;
    border-radius: 50%;
    font-size: 13px;
    line-height: 1;
    color: var(--text-dim);
    cursor: pointer;
  }
  .tag-remove:hover {
    color: var(--red);
    background: var(--bg);
  }
  .tag-text-input {
    flex: 1;
    min-width: 60px;
    border: none;
    outline: none;
    background: transparent;
    font-size: 13px;
    font-family: var(--sans);
    color: var(--text);
    padding: 2px 0;
  }
  .tag-text-input::placeholder {
    color: var(--text-dim);
  }
</style>

#!/usr/bin/env bun
// GitHub Pages へ公開するページを生成する。
// 公開するのはプライバシーポリシーだけで、本文の正本は docs/privacy-policy.md とする。
// HTML はこの正本から都度生成し、リポジトリへはコミットしない。
// 生成物は site/ へ出力し、.github/workflows/pages.yml が公開する。

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const sourcePath = join(repositoryRoot, "docs", "privacy-policy.md");
const outputDirectory = join(repositoryRoot, "site");

const PAGE_TITLE = "プライバシーポリシー / Privacy Policy — clp2cfg";

const HTML_ENTITIES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
  "&#x27;": "'",
};

function decodeHtmlText(html: string): string {
  const withoutTags = html.replace(/<[^>]+>/g, "");
  return withoutTags.replace(
    /&(?:amp|lt|gt|quot|#39|#x27);/g,
    (entity) => HTML_ENTITIES[entity] ?? entity,
  );
}

/** GitHub の見出しアンカー（github-slugger）互換の ID を生成する。 */
export function slugifyHeading(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}_\- ]/gu, "")
    .replace(/ /g, "-");
}

function addHeadingIds(html: string): string {
  const used = new Map<string, number>();
  return html.replace(/<(h[1-6])>([\s\S]*?)<\/\1>/g, (_match, tag: string, inner: string) => {
    const base = slugifyHeading(decodeHtmlText(inner));
    const count = used.get(base) ?? 0;
    used.set(base, count + 1);
    const id = count === 0 ? base : `${base}-${count}`;
    return `<${tag} id="${id}">${inner}</${tag}>`;
  });
}

// 英語の節（見出し「English」から末尾まで）を `lang="en"` の section で囲む。
// スクリーンリーダーが読み上げの言語を切り替えられる。文書全体は `lang="ja"` とする。
function wrapEnglishSection(html: string): string {
  const marker = '<h2 id="english">';
  const start = html.indexOf(marker);
  if (start === -1) {
    throw new Error("英語の節（## English）が見つかりません");
  }
  const before = html.slice(0, start);
  const english = html.slice(start);
  return `${before}<section lang="en">\n${english}</section>\n`;
}

const STYLE = `
:root { color-scheme: light dark; }
body {
  margin: 0 auto;
  padding: 2rem 1rem 4rem;
  max-width: 46rem;
  line-height: 1.75;
  font-family: system-ui, -apple-system, "Segoe UI", "Yu Gothic UI", "Meiryo UI", sans-serif;
}
h1, h2, h3 { line-height: 1.3; }
h2 { margin-top: 2.5rem; padding-bottom: .3rem; border-bottom: 1px solid color-mix(in srgb, currentColor 25%, transparent); }
table { border-collapse: collapse; width: 100%; margin: 1rem 0; }
th, td { border: 1px solid color-mix(in srgb, currentColor 25%, transparent); padding: .4rem .6rem; text-align: left; vertical-align: top; }
code { font-family: ui-monospace, "Cascadia Mono", Consolas, monospace; font-size: .92em; }
hr { border: 0; border-top: 1px solid color-mix(in srgb, currentColor 25%, transparent); margin: 2.5rem 0; }
a { overflow-wrap: anywhere; }
`.trim();

/** プライバシーポリシーの Markdown から、公開する HTML 文書を作る。 */
export function renderPrivacyPolicyPage(markdown: string): string {
  // 生の HTML は通さない（正本は Markdown だけで書く）。
  const rawHtml = Bun.markdown.html(markdown, {
    tables: true,
    noHtmlBlocks: true,
    noHtmlSpans: true,
  });
  const body = wrapEnglishSection(addHeadingIds(rawHtml));

  return `<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${PAGE_TITLE}</title>
<style>
${STYLE}
</style>
</head>
<body>
<main>
${body}</main>
</body>
</html>
`;
}

/** ルートの URL でも到達できるよう、ポリシーのページへ転送する。 */
export function renderIndexPage(): string {
  return `<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta http-equiv="refresh" content="0; url=privacy-policy.html">
<link rel="canonical" href="privacy-policy.html">
<title>${PAGE_TITLE}</title>
</head>
<body>
<p><a href="privacy-policy.html">プライバシーポリシー / Privacy Policy</a></p>
</body>
</html>
`;
}

if (import.meta.main) {
  const markdown = readFileSync(sourcePath, "utf-8");
  mkdirSync(outputDirectory, { recursive: true });
  writeFileSync(
    join(outputDirectory, "privacy-policy.html"),
    renderPrivacyPolicyPage(markdown),
    "utf-8",
  );
  writeFileSync(join(outputDirectory, "index.html"), renderIndexPage(), "utf-8");
  console.log(`${outputDirectory} へ privacy-policy.html と index.html を生成しました`);
}

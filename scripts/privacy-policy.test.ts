import { describe, expect, test } from "bun:test";
import { renderIndexPage, renderPrivacyPolicyPage, slugifyHeading } from "./build-pages";

// プライバシーポリシー（docs/privacy-policy.md）が、実装の永続化キーや
// HTML 出力要件（日英の章・Windows エラー報告・XSS 無効化）と整合することを固定する。

const policy = await Bun.file(new URL("../docs/privacy-policy.md", import.meta.url)).text();
const systemRs = await Bun.file(
  new URL("../crates/clp2cfg/src/app/system.rs", import.meta.url),
).text();

const storageKeys = [
  ...systemRs.matchAll(/pub const \w+_STORAGE_KEY:\s*&str\s*=\s*"([^"]+)"/g),
].map((m) => m[1] ?? "");

describe("プライバシーポリシー", () => {
  test("system.rs の永続化キーを抽出できている", () => {
    expect(storageKeys).toEqual(["auto_read_clipboard"]);
  });

  test("永続化キーが日英の両セクションに記載されている", () => {
    const [japanese = "", english = ""] = policy.split(/^## English$/m);
    for (const key of storageKeys) {
      expect(japanese).toContain(`\`${key}\``);
      expect(english).toContain(`\`${key}\``);
    }
  });

  test.each([
    ["日本語", 0, "Windows エラー報告"],
    ["English", 1, "Windows Error Reporting"],
  ] as const)("Windows の診断情報についての説明を %s に載せている", (_language, index, term) => {
    const section = policy.split(/^## English$/m)[index] ?? "";
    expect(section).toContain(term);
  });

  test.each([
    ["日本語", "日本語"],
    ["English", "english"],
    ["プライバシーポリシー / Privacy Policy", "プライバシーポリシー--privacy-policy"],
    ["開発者が収集する情報", "開発者が収集する情報"],
  ] as const)("見出し slug が GitHub 互換: %s", (heading, expected) => {
    expect(slugifyHeading(heading)).toBe(expected);
  });

  test('生成 HTML に日英のアンカーと lang="en" セクションが含まれる', () => {
    const html = renderPrivacyPolicyPage(policy);
    expect(html).toContain('<html lang="ja">');
    expect(html).toContain('<h2 id="日本語">日本語</h2>');
    expect(html).toContain('<section lang="en">\n<h2 id="english">English</h2>');
    expect(html).toContain("<code>auto_read_clipboard</code>");
  });

  test("生の HTML はエスケープされ、## English 欠落時はエラーになる", () => {
    const safeHtml = renderPrivacyPolicyPage(
      "# タイトル\n\n<script>alert(1)</script>\n\n## English\n\nText",
    );
    expect(safeHtml).not.toContain("<script>");
    expect(safeHtml).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");

    expect(() => renderPrivacyPolicyPage("# Title\n\nNo English")).toThrow();
  });

  test("index.html は privacy-policy.html へリダイレクトする", () => {
    const indexHtml = renderIndexPage();
    expect(indexHtml).toContain('url=privacy-policy.html"');
    expect(indexHtml).toContain('href="privacy-policy.html"');
  });
});

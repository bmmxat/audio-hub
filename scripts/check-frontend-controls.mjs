import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const sources = [
    'frontend/index.html',
    'frontend/js/app.js',
];
const css = readFileSync(resolve(root, 'frontend/css/style.css'), 'utf8');
const failures = [];

function hasCssClass(className) {
    const escaped = className.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    return new RegExp(`\\.${escaped}(?![\\w-])`).test(css);
}

for (const source of sources) {
    const contents = readFileSync(resolve(root, source), 'utf8');
    const maskedContents = contents.replace(
        /\$\{[\s\S]*?\}/g,
        (expression) => expression.replace(/[^\r\n]/g, ' '),
    );
    const buttonTags = maskedContents.matchAll(/<button\b[^>]*>/gs);

    for (const match of buttonTags) {
        const tag = match[0];
        const line = maskedContents.slice(0, match.index).split(/\r?\n/).length;
        const classMatch = tag.match(/class="([^"]*)"/s);
        if (!classMatch) {
            failures.push(`${source}:${line} button 缺少 class 属性`);
            continue;
        }

        const staticClasses = classMatch[1]
            .split(/\s+/)
            .filter((className) => /^[a-z][\w-]*$/i.test(className));

        if (staticClasses.length === 1 && staticClasses[0] === 'btn') {
            failures.push(`${source}:${line} button 只有基础 btn，缺少明确的视觉样式类`);
        }

        for (const className of staticClasses) {
            if (!hasCssClass(className)) {
                failures.push(`${source}:${line} 未找到按钮样式类 .${className}`);
            }
        }
    }

    const selectTags = maskedContents.matchAll(/<select\b[^>]*>/gs);
    for (const match of selectTags) {
        const tag = match[0];
        const line = maskedContents.slice(0, match.index).split(/\r?\n/).length;
        const classMatch = tag.match(/class="([^"]*)"/s);
        if (!classMatch) {
            failures.push(`${source}:${line} select 缺少 class 属性`);
            continue;
        }

        const staticClasses = classMatch[1]
            .split(/\s+/)
            .filter((className) => /^[a-z][\w-]*$/i.test(className));
        for (const className of staticClasses) {
            if (!hasCssClass(className)) {
                failures.push(`${source}:${line} 未找到选择框样式类 .${className}`);
            }
        }
    }
}

if (!/button\s*\{[\s\S]*?appearance:\s*none/.test(css)) {
    failures.push('style.css 缺少 button 的 appearance: none 全局基线');
}
if (!/input\[type="checkbox"\][\s\S]*?accent-color:\s*var\(--accent\)/.test(css)) {
    failures.push('style.css 缺少 checkbox 的强调色基线');
}
if (!/input\[type="range"\][\s\S]*?accent-color:\s*var\(--accent\)/.test(css)) {
    failures.push('style.css 缺少 range 的强调色基线');
}

if (failures.length > 0) {
    console.error('前端控件样式检查失败：');
    for (const failure of failures) console.error(`- ${failure}`);
    process.exitCode = 1;
} else {
    console.log('前端控件样式检查通过。');
}

#!/bin/bash
# i18n-doc-fix.sh - 自动修复可识别的问题
# 使用: ./scripts/i18n-doc-fix.sh [--dry-run]

set -euo pipefail

# 安全地收集所有 zh-CN.md 文件（兼容 bash 3.2，支持嵌套目录和空格）
zh_files=()
while IFS= read -r -d '' f; do
  zh_files+=("$f")
done < <(find gitbooks -type f -name '*.zh-CN.md' -print0)

DRY_RUN=false
if [[ "$1" == "--dry-run" ]]; then
  DRY_RUN=true
  echo "🔍 Dry-run 模式，仅显示将要修改的内容"
  echo ""
fi

# 1. 修复裸代码块（``` → ```text）
echo "【1/4】修复裸代码块..."
for f in "${zh_files[@]}"; do
  # 匹配孤立的 ``` 行（前后不是 ```text 这样的语言标识）
  # 简单策略：在 ``` 后紧跟非字母字符的改为 ```text
  if grep -q '^```$' "$f"; then
    if $DRY_RUN; then
      echo "   [dry-run] would fix: $f"
    else
      # stateful 处理：只给 opening fence 加 text，closing fence 保持原样
      perl -i -pe '
        if (/^```$/) {
          if ($in_block) {
            $in_block = 0;
          } else {
            $_ = "```text";
            $in_block = 1;
          }
        }
      ' "$f"
      echo "   fixed: $f"
    fi
  fi
done
echo ""

# 2. 修复 http:// → https://（只改外部域名链接，不改内部路径）
echo "【2/4】修复 http:// → https://..."
for f in "${zh_files[@]}"; do
  if grep -q 'http://' "$f"; then
    if $DRY_RUN; then
      echo "   [dry-run] would fix: $f"
    else
      # 只替换 http:// 开头且后面不是 // 开头的（避免把 //path 变成 https:////path）
      perl -i -pe 's|http://(?![/])|https://|g' "$f"
      echo "   fixed: $f"
    fi
  fi
done
echo ""

# 3. 修复 sidecar 术语
echo "【3/4】移除 sidecar 术语（core 已内联）..."
for f in "${zh_files[@]}"; do
  if grep -qi 'sidecar' "$f"; then
    if $DRY_RUN; then
      echo "   [dry-run] would fix: $f"
    else
      # 替换 sidecar 相关描述为更准确的说法
      perl -i -pe 's/\bsidecar\b/in-process core/gi' "$f"
      echo "   fixed: $f"
    fi
  fi
done
echo ""

# 4. 添加末尾空行
echo "【4/4】确保文件末尾有空行..."
for f in "${zh_files[@]}"; do
  last=$(tail -c1 "$f" 2>/dev/null | xxd -p)
  if [[ "$last" != "0a" && -s "$f" ]]; then
    if $DRY_RUN; then
      echo "   [dry-run] would fix: $f"
    else
      echo "" >> "$f"
      echo "   fixed: $f"
    fi
  fi
done
echo ""

$DRY_RUN && echo "✅ Dry-run 完成，使用不带 --dry-run 参数运行以实际修改。" || echo "✅ 修复完成。"

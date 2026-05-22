#!/bin/bash
# i18n-doc-scan.sh - OpenHuman GitBook 中文文档问题扫描

set -e

echo "=== OpenHuman i18n 文档扫描 ==="
echo ""

# 1. 未本地化的内部链接
echo "【1/5】检查未本地化的 .md 链接..."
UNLOCALIZED=$(find gitbooks -name "*.zh-CN.md" -exec grep -l '\.md)' {} \; 2>/dev/null | while read f; do
  grep '\.md)' "$f" 2>/dev/null | grep -v '\.zh-CN\.md)' | sed "s|^|$f:|" || true
done)
if [[ -n "$UNLOCALIZED" ]]; then
  COUNT=$(echo "$UNLOCALIZED" | grep -c ':' || echo 0)
  echo "❌ 发现未本地化链接（共 $COUNT 处）："
  echo "$UNLOCALIZED" | head -30
else
  echo "✅ 无未本地化链接"
fi
echo ""

# 2. MD040 - 代码块缺少语言标识（查找孤立的 ``` 行）
echo "【2/5】检查代码块语言标识（MD040）..."
NO_LANG=$(find gitbooks -name "*.zh-CN.md" -exec sh -c '
  for f; do
    line_num=0
    while IFS= read -r line; do
      line_num=$((line_num + 1))
      if [[ "$line" == "\`\`\`" ]]; then
        # 检查前一行是否也是 ``` 或空（是代码块开始）
        # 简单判断：当前行是 ``` 且下一行不是以 ``` 开头（结尾没有语言标识）
        prev_line=$(sed "$((line_num - 1))q;d" "$f" 2>/dev/null || echo "")
        next_line=$(sed "$((line_num + 1))q;d" "$f" 2>/dev/null || echo "")
        if [[ ! "$line" =~ ^\`\`\`[a-zA-Z] ]]; then
          echo "$f:$line_num: $line"
        fi
      fi
    done < "$f"
  done
' sh {} + 2>/dev/null || true)
if [[ -n "$NO_LANG" && ${#NO_LANG} -gt 10 ]]; then
  echo "❌ 发现裸代码块："
  echo "$NO_LANG" | head -20
else
  echo "✅ 所有代码块均有语言标识"
fi
echo ""

# 3. http:// 外部链接
echo "【3/5】检查 http:// 外部链接..."
HTTP_FILES=$(find gitbooks -name "*.zh-CN.md" -exec grep -l 'http://' {} \; 2>/dev/null || true)
if [[ -n "$HTTP_FILES" ]]; then
  echo "❌ 发现 http:// 链接："
  find gitbooks -name "*.zh-CN.md" -exec grep -n 'http://' {} \; 2>/dev/null | head -10
else
  echo "✅ 无 http:// 链接"
fi
echo ""

# 4. sidecar 术语
echo "【4/5】检查 sidecar 术语..."
SIDECAR_FILES=$(find gitbooks -name "*.zh-CN.md" -exec grep -l -i 'sidecar' {} \; 2>/dev/null || true)
if [[ -n "$SIDECAR_FILES" ]]; then
  echo "❌ 发现 sidecar 提及："
  find gitbooks -name "*.zh-CN.md" -exec grep -n -i 'sidecar' {} \; 2>/dev/null | head -10
else
  echo "✅ 无 sidecar 术语"
fi
echo ""

# 5. 末尾空行检查
echo "【5/5】检查文件末尾空行..."
MISSING_TRAILING=$(find gitbooks -name "*.zh-CN.md" -exec sh -c '
  for f; do
    if [[ -s "$f" ]]; then
      last=$(tail -c1 "$f" 2>/dev/null | xxd -p | tr -d " ")
      if [[ "$last" != "0a" ]]; then
        echo "$f"
      fi
    fi
  done
' sh {} + 2>/dev/null || true)
if [[ -n "$MISSING_TRAILING" ]]; then
  echo "❌ 文件缺少末尾空行（共 $(echo "$MISSING_TRAILING" | wc -l) 个）："
  echo "$MISSING_TRAILING" | head -10
else
  echo "✅ 所有文件末尾有空行"
fi
echo ""

echo "=== 扫描完成 ==="
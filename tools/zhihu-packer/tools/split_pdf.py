import argparse
import os
import re
import fitz  # PyMuPDF

def clean_filename(title):
    # 去除空字符和控制字符
    title = "".join(ch for ch in title if ch != '\x00')
    # 替换 Windows 文件名中的不合法字符
    # \ / : * ? " < > |
    rstr = r"[\/\\\:\*\?\"\<\>\|]"
    new_title = re.sub(rstr, "_", title)
    # 缩短超长标题，避免文件名超长
    if len(new_title) > 60:
        new_title = new_title[:60] + "..."
    return new_title.strip()

def rebuild_paragraph(block):
    # 把文本块内部的所有行优雅地拼接成一个长段落
    lines_text = []
    for line in block.get("lines", []):
        line_bbox = line.get("bbox", (0, 0, 0, 0))
        span_text = "".join(span.get("text", "") for span in line.get("spans", []))
        s_text = span_text.strip()
        
        if not s_text:
            continue
            
        # 过滤顶部和底部的页码数字
        # 如果是纯数字，且位于页面最顶端 (y1 < 50) 或页面最底端 (y1 > 740)
        if s_text.isdigit():
            if line_bbox[3] < 50 or line_bbox[3] > 740:
                continue
                
        lines_text.append(s_text)
        
    paragraph = ""
    for idx, line_text in enumerate(lines_text):
        if not line_text:
            continue
        if not paragraph:
            paragraph = line_text
        else:
            # 判断前一行的最后一个字符与下一行的第一个字符是否需要留空格
            # 如果都是中文，无缝拼接；否则加一个空格
            last_char = paragraph[-1]
            first_char = line_text[0]
            if re.match(r'[\u4e00-\u9fa5]', last_char) and re.match(r'[\u4e00-\u9fa5]', first_char):
                paragraph += line_text
            else:
                paragraph += " " + line_text
    return paragraph.strip()

def process_pdf(pdf_path, output_dir, author_name="知乎作者", archive_title=None):
    archive_title = archive_title or author_name
    print(f"Opening PDF: {pdf_path}")
    doc = fitz.open(pdf_path)
    total_pages = len(doc)
    print(f"Total pages: {total_pages}")
    
    # 准备输出目录
    os.makedirs(output_dir, exist_ok=True)
    images_dir = os.path.join(output_dir, "images")
    os.makedirs(images_dir, exist_ok=True)
    
    all_blocks = []
    image_counter = 1
    
    # 1. 全局扫描所有页面，构建扁平的 all_blocks 列表并保存图片
    print("Scanning PDF pages and extracting blocks...")
    for page_num in range(total_pages):
        page = doc[page_num]
        page_dict = page.get_text("dict")
        page_blocks = page_dict.get("blocks", [])
        
        for block in page_blocks:
            btype = block.get("type", 0)
            bbox = block.get("bbox", (0,0,0,0))
            
            if btype == 0:
                # 文本块
                # 将文本块里的 spans 拼接为单行文本以做页眉页脚的检测
                lines_text = []
                for line in block.get("lines", []):
                    # 对单行进行页码过滤
                    line_bbox = line.get("bbox", (0, 0, 0, 0))
                    span_text = "".join(span.get("text", "") for span in line.get("spans", []))
                    s_text = span_text.strip()
                    if s_text.isdigit() and (line_bbox[3] < 50 or line_bbox[3] > 740):
                        continue
                    if s_text:
                        lines_text.append(s_text)
                        
                full_text = " ".join(lines_text).strip()
                
                if not full_text:
                    continue
                
                # 过滤合集页眉
                if full_text == archive_title or full_text == author_name:
                    continue
                
                # 再次过滤单纯的数字页码
                if full_text.isdigit() and (bbox[3] < 50 or bbox[3] > 740):
                    continue
                
                all_blocks.append({
                    'type': 'text',
                    'text': full_text,
                    'block_data': block,
                    'page': page_num
                })
                
            elif btype == 1:
                # 图像块
                img_bytes = block.get("image")
                ext = block.get("ext", "png")
                if img_bytes:
                    img_filename = f"image_{image_counter}.{ext}"
                    img_path = os.path.join(images_dir, img_filename)
                    with open(img_path, "wb") as f:
                        f.write(img_bytes)
                        
                    all_blocks.append({
                        'type': 'image',
                        'path': f"images/{img_filename}",
                        'page': page_num
                    })
                    image_counter += 1
                    
    print(f"Extraction completed. Total clean blocks: {len(all_blocks)}, Total images: {image_counter - 1}")
    
    # 2. 定位所有文章的篇章起点
    print("Indexing articles from block list...")
    articles = []
    for idx, block in enumerate(all_blocks):
        if block['type'] == 'text':
            match = re.search(r'第\s*(\d+)\s*篇\s*:\s*(https?://[^\s]+)', block['text'])
            if match:
                part_num = int(match.group(1))
                link = match.group(2)
                
                # 提取 Answer / Post / Pin ID
                ans_id_match = re.search(r'/answer/(\d+)|/p/(\d+)|/pin/(\d+)', link)
                ans_id = "unknown"
                if ans_id_match:
                    ans_id = ans_id_match.group(1) or ans_id_match.group(2) or ans_id_match.group(3)
                
                if ans_id == "unknown":
                    ans_id = f"part_{part_num}"
                    
                articles.append({
                    'part_num': part_num,
                    'ans_id': ans_id,
                    'link': link,
                    'start_idx': idx,
                    'page': block['page']
                })
                
    # 补全每篇文章的结束块索引
    for idx in range(len(articles)):
        if idx + 1 < len(articles):
            articles[idx]['end_idx'] = articles[idx+1]['start_idx'] - 1
        else:
            articles[idx]['end_idx'] = len(all_blocks) - 1
            
    print(f"Total articles index mapped: {len(articles)}")
    
    processed_articles = []
    
    # 3. 遍历每一篇文章，提取元数据、重建正文并保存
    for idx, art in enumerate(articles):
        part_num = art['part_num']
        ans_id = art['ans_id']
        start_idx = art['start_idx']
        end_idx = art['end_idx']
        
        # 寻找这一篇里的元数据块（赞同数和时间所在块）
        meta_idx = -1
        for k in range(start_idx + 1, min(start_idx + 6, end_idx + 1)):
            if all_blocks[k]['type'] == 'text' and ("赞同数" in all_blocks[k]['text'] or "创建时间" in all_blocks[k]['text']):
                meta_idx = k
                break
                
        # 提取标题
        title = ""
        upvotes = 0
        created_date = "2026-05-24"
        
        if meta_idx != -1:
            title_parts = []
            for k in range(start_idx + 1, meta_idx):
                if all_blocks[k]['type'] == 'text':
                    title_parts.append(all_blocks[k]['text'])
            title = " ".join(title_parts).strip()
            
            # 解析发布日期和赞同数
            meta_text = all_blocks[meta_idx]['text']
            
            upvote_match = re.search(r'赞同数:\s*\(\s*(\d+)\s*赞同\)', meta_text)
            if upvote_match:
                upvotes = int(upvote_match.group(1))
            date_match = re.search(r'创建时间:\s*\(\s*([\d-]+)\s*\)', meta_text)
            if date_match:
                created_date = date_match.group(1)
                
            # 优化：提取元数据块中可能夹带的标题前缀（例如：给00后们的话赞同数:...）
            meta_title_match = re.search(r'^(.*?)\s*赞同数\s*:', meta_text)
            if meta_title_match:
                prefix_title = meta_title_match.group(1).strip()
                if prefix_title:
                    # 清理并合并标题
                    if not title or title.isdigit() or title == str(part_num):
                        title = prefix_title
                    else:
                        title = f"{title} {prefix_title}"
        else:
            # 容错：没找到元数据块时的默认值
            upvotes = 0
            created_date = "2026-05-24"
            title = ""
            
        # 确定正文块的起始位置
        body_start_idx = meta_idx + 1 if meta_idx != -1 else start_idx + 1
        
        # 构建正文内容列表
        body_paragraphs = []
        for k in range(body_start_idx, end_idx + 1):
            block = all_blocks[k]
            if block['type'] == 'text':
                para_text = rebuild_paragraph(block['block_data'])
                if para_text:
                    body_paragraphs.append(para_text)
            elif block['type'] == 'image':
                # 图像块，直接追加 Markdown 图像引用
                body_paragraphs.append(f"![image]({block['path']})")
                
        # 如果标题为空，用正文第一段的前 30 个字代替
        if not title:
            # 寻找第一个非图片文本段落
            first_text_para = ""
            for p in body_paragraphs:
                if not p.startswith("![image]"):
                    first_text_para = p
                    break
            if first_text_para:
                title = first_text_para[:30] + ("..." if len(first_text_para) > 30 else "")
            else:
                title = f"无标题想法 {part_num}"
                
        # 格式化输出的 Markdown 内容
        # 将空字符等非法字符做替换
        title_clean = title.replace('\x00', '').strip()
        # 清理多余的双空格
        title_clean = re.sub(r'\s+', ' ', title_clean)
        ans_id_clean = ans_id.replace('\x00', '').strip()
        
        md_lines = []
        md_lines.append("---\n")
        md_lines.append(f"title: {title_clean}\n")
        md_lines.append(f"author: {author_name}\n")
        md_lines.append(f"date: {created_date}\n")
        md_lines.append(f"voteup_count: {upvotes}\n")
        md_lines.append("---\n\n")
        
        for para in body_paragraphs:
            para_clean = para.replace('\x00', '')
            md_lines.append(para_clean + "\n\n")
                
        final_md_text = "".join(md_lines)
        
        # 安全的文件名
        safe_title = clean_filename(title_clean)
        filename = f"{created_date}-{safe_title}_{ans_id_clean}.md"
        file_path = os.path.join(output_dir, filename)
        
        with open(file_path, "w", encoding="utf-8") as f:
            f.write(final_md_text)
            
        processed_articles.append({
            'filename': filename,
            'title': title_clean,
            'date': created_date,
            'upvotes': upvotes,
            'part_num': part_num
        })
        
        if (idx + 1) % 50 == 0 or idx + 1 == len(articles):
            print(f"  Processed {idx + 1}/{len(articles)} articles...")
            
    # 4. 生成 index.md 索引，以日期和篇号进行降序排列
    processed_articles.sort(key=lambda x: (x['date'], x['part_num']), reverse=True)
    
    index_path = os.path.join(output_dir, "index.md")
    print(f"Generating index.md: {index_path}")
    
    index_lines = []
    index_lines.append(f"# {author_name} 的内容归档\n\n")
    index_lines.append("> 本归档由 Zhihu Packer 自动生成。  \n")
    index_lines.append(f"> 共归档回答: **{len(processed_articles)}** 篇，文章: **0** 篇。  \n\n")
    index_lines.append("## 回答列表\n\n")
    
    for art in processed_articles:
        index_lines.append(f"- [[{art['filename']}|{art['title']}]] (发布于: {art['date']} | 赞同数: {art['upvotes']})\n")
        
    with open(index_path, "w", encoding="utf-8") as f:
        f.write("".join(index_lines))
        
    print(f"PDF processing completed successfully! Total {len(processed_articles)} articles processed.")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Split a Zhihu archive PDF into Markdown files and extracted images."
    )
    parser.add_argument("--input", required=True, help="Path to the source PDF file.")
    parser.add_argument("--output", required=True, help="Directory where Markdown files and images will be written.")
    parser.add_argument("--author", default="知乎作者", help="Author name used in frontmatter and index title.")
    parser.add_argument("--archive-title", help="PDF header text to ignore. Defaults to the author name.")
    args = parser.parse_args()

    pdf_path = os.path.abspath(args.input)
    output_dir = os.path.abspath(args.output)
    process_pdf(pdf_path, output_dir, author_name=args.author, archive_title=args.archive_title)

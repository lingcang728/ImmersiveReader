import * as fs from 'fs';
import * as path from 'path';
import * as esbuild from 'esbuild';

const __dirname = path.resolve();

// 依赖路径定义
const templatePath = path.join(__dirname, 'src', 'reader-template.html');
const markedPath = path.join(__dirname, 'node_modules', 'marked', 'lib', 'marked.umd.js');
const purifyPath = path.join(__dirname, 'node_modules', 'dompurify', 'dist', 'purify.min.js');

const distDir = path.join(__dirname, 'dist');
const outputDir = path.join(__dirname, 'output');

// 编译输出的目标模板 (用于给知乎打包脚本 build-html.ts 读取)
const distTemplatePath = path.join(distDir, 'reader-template.html');

// 编译输出的通用独立阅读器 (供用户拷贝复用)
const universalReaderPath = path.join(outputDir, 'universal-reader.html');

async function compile() {
  console.log('📦 开始打包编译通用 Markdown 阅读器前端...');

  // 1. 验证必要依赖文件是否存在
  if (!fs.existsSync(templatePath)) {
    console.error(`❌ HTML 模板不存在: ${templatePath}`);
    process.exitCode = 1;
    return;
  }
  if (!fs.existsSync(markedPath)) {
    console.error(`❌ marked.min.js 未找到，请确保已安装 marked 依赖: ${markedPath}`);
    process.exitCode = 1;
    return;
  }
  if (!fs.existsSync(purifyPath)) {
    console.error(`❌ purify.min.js 未找到，请确保已安装 dompurify 依赖: ${purifyPath}`);
    process.exitCode = 1;
    return;
  }

  // 创建必要目录
  if (!fs.existsSync(distDir)) fs.mkdirSync(distDir, { recursive: true });
  if (!fs.existsSync(outputDir)) fs.mkdirSync(outputDir, { recursive: true });

  try {
    // 2. 读取并转义第三方库，杜绝 HTML 标签提前截断风险
    const escapeScriptTag = (code: string) => code.replace(/<\/script>/gi, '<\\/script>');
    
    console.log('📖 读取并转义第三方核心库...');
    const markedJs = escapeScriptTag(fs.readFileSync(markedPath, 'utf-8'));
    const purifyJs = escapeScriptTag(fs.readFileSync(purifyPath, 'utf-8'));
    const htmlTemplate = fs.readFileSync(templatePath, 'utf-8');

    // 3. 使用 esbuild 快速 Bundle 模块化 TypeScript 前端源码
    console.log('⚡ 使用 esbuild 编译前端 TypeScript 源码...');
    const buildResult = await esbuild.build({
      entryPoints: [path.join(__dirname, 'src', 'reader', 'index.ts')],
      bundle: true,
      minify: true,
      write: false,
      format: 'iife', // 浏览器端直接自执行
      target: 'es2022',
      logLevel: 'info',
    });

    if (!buildResult.outputFiles || buildResult.outputFiles.length === 0) {
      throw new Error('esbuild 编译输出文件为空');
    }

    const frontendBundleJs = escapeScriptTag(buildResult.outputFiles[0].text);

    // 4. 将所有第三方库及前端 bundle 注入到 HTML 骨架中
    console.log('✍️ 注入 JS 源码至 HTML 骨架...');
    const compiledHtml = htmlTemplate
      .replace('/* MARKED_JS_PLACEHOLDER */', markedJs)
      .replace('/* DOMPURIFY_JS_PLACEHOLDER */', purifyJs)
      .replace('/* FRONTEND_BUNDLE_PLACEHOLDER */', frontendBundleJs);

    // 5. 输出编译结果
    // 输出 dist 模板 (让 build-html.ts 后续作为知乎打包数据源)
    fs.writeFileSync(distTemplatePath, compiledHtml, 'utf-8');
    console.log(`✅ 成功输出分发版模板 (Zhihu Packer 依赖): ${distTemplatePath}`);

    // 输出干净的通用阅读器 (没有任何注入 JSON 数据的版本)
    // 确保没有误带临时或测试 JSON，替换 JSON 占位符为空
    const cleanUniversalHtml = compiledHtml
      .replace('/* ARTICLES_JSON_PLACEHOLDER */', '[]')
      .replace('<!-- ARTICLES_DOM_PLACEHOLDER -->', '');

    fs.writeFileSync(universalReaderPath, cleanUniversalHtml, 'utf-8');
    console.log(`✅ 成功输出独立的通用阅读器: ${universalReaderPath}`);
    
    console.log('🎉 前端工程构建圆满成功！');
  } catch (err) {
    console.error('❌ 打包通用阅读器失败:', err);
    process.exitCode = 1;
  }
}

compile();

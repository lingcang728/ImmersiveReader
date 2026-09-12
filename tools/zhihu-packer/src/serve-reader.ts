import express from 'express';
import * as path from 'path';

const app = express();
// P3-28：不暴露 Express 指纹（与 sidecar server.ts 一致）。
app.disable('x-powered-by');
const port = Number(process.env.ZHIHU_READER_PORT) || 3080;
// P3-28：默认只绑回环地址 —— output/ 里是私人归档内容，绑 0.0.0.0 会把整个
// 归档暴露给局域网。确需局域网预览时显式传 --lan 或 ZHIHU_READER_HOST=0.0.0.0。
const host = process.env.ZHIHU_READER_HOST || (process.argv.includes('--lan') ? '0.0.0.0' : '127.0.0.1');
const __dirname = path.resolve();
const outputDir = path.join(__dirname, 'output');

// 静态托管 output 目录下的文件
app.use(express.static(outputDir));

console.log('📂 正在静态托管目录:', outputDir);

app.listen(port, host, () => {
  console.log('\n======================================================');
  console.log('🚀 沉浸式 Markdown 阅读器本地托管服务启动成功！');
  console.log('======================================================');
  console.log(`👉 通用独立阅读器：http://${host === '0.0.0.0' ? 'localhost' : host}:${port}/universal-reader.html`);
  console.log(`👉 答主归档阅读器：http://${host === '0.0.0.0' ? 'localhost' : host}:${port}/reader.html`);
  if (host !== '0.0.0.0') {
    console.log('ℹ️  仅监听本机回环地址；如需局域网访问请加 --lan 或设置 ZHIHU_READER_HOST。');
  } else {
    console.warn('⚠️  正在监听 0.0.0.0 —— 局域网内所有设备均可访问归档内容！');
  }
  console.log('======================================================\n');
});
export {};

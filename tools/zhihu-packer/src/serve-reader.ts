import express from 'express';
import * as path from 'path';

const app = express();
const port = 3080;
const __dirname = path.resolve();
const outputDir = path.join(__dirname, 'output');

// 静态托管 output 目录下的文件
app.use(express.static(outputDir));

console.log('📂 正在静态托管目录:', outputDir);

app.listen(port, () => {
  console.log('\n======================================================');
  console.log('🚀 沉浸式 Markdown 阅读器本地托管服务启动成功！');
  console.log('======================================================');
  console.log(`👉 通用独立阅读器：http://localhost:${port}/universal-reader.html`);
  console.log(`👉 答主归档阅读器：http://localhost:${port}/reader.html`);
  console.log('======================================================\n');
});
export {};

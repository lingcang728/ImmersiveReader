import { Command } from 'commander';
import { runDoctor } from './doctor.js';
import { runLogin } from './login.js';
import { getBrowserContext, closeBrowserContext } from './browser.js';
import { scrapeAnswer, scrapeArticle, writeMarkdownFile } from './extractor.js';
import { initDb, saveItem, getTasks, saveTask, getTask, resetTaskForce } from './db.js';
import { logger, sanitizeFilename } from './utils.js';
import * as path from 'path';
import * as fs from 'fs';
import { createTask, runTask } from './scheduler.js';
import { scanLimitForSelection, scrapePeopleIndex, selectIndexItems } from './indexer.js';
import { startServer } from './server.js';
import { resolveArchiveOutputDir } from './runtime-paths.js';

const program = new Command();
const defaultOutputDir = resolveArchiveOutputDir({ cwd: process.cwd(), environment: process.env });

program
  .name('zhihu-packer')
  .description('知乎回答和文章的 Obsidian 备份/归档工具')
  .version('1.0.0');

initDb();

program
  .command('doctor')
  .description('执行环境自检')
  .action(async () => {
    try {
      const ok = await runDoctor();
      process.exitCode = ok ? 0 : 1;
    } catch (e: any) {
      logger.error(`自检出错: ${e.message}`);
      process.exitCode = 1;
    }
  });

program
  .command('login')
  .description('显式开启有头浏览器手动登录知乎并保存登录态')
  .action(async () => {
    try {
      await runLogin();
      process.exitCode = 0;
    } catch (e: any) {
      logger.error(`登录出错: ${e.message}`);
      process.exitCode = 1;
    }
  });

program
  .command('save-answer <url>')
  .description('抓取并保存单个知乎回答')
  .option('-o, --output <dir>', '指定输出根目录', defaultOutputDir)
  .option('-d, --dry-run', '预览模式，不实际写入文件和数据库')
  .action(async (url, options) => {
    logger.info(`开始抓取回答: ${url}`);
    let context;
    try {
      context = await getBrowserContext(true);
      const page = await context.newPage();
      
      const extracted = await scrapeAnswer(page, url);

      if (options.dryRun) {
        logger.info('=== [DRY RUN 预览] ===');
        logger.info(`类型: 回答`);
        logger.info(`标题: ${extracted.title}`);
        logger.info(`作者: ${extracted.authorName}`);
        logger.info(`发布时间: ${new Date(extracted.createdTime * 1000).toLocaleString()}`);
        logger.info(`赞同数: ${extracted.voteupCount}`);
        logger.info(`正文预览 (前 200 字):\n${extracted.contentMarkdown.slice(0, 200)}...`);
        logger.info('=======================');
        process.exitCode = 0;
        return;
      }

      const outputDir = path.resolve(process.cwd(), options.output);
      const filePath = await writeMarkdownFile(extracted, outputDir);

      saveItem({
        id: extracted.id,
        item_type: 'answer',
        author_id: extracted.authorId,
        author_name: extracted.authorName,
        title: extracted.title,
        answer_id: extracted.answerId || null,
        question_id: extracted.questionId || null,
        article_id: null,
        url: extracted.url,
        question_url: extracted.questionUrl || null,
        created_time: extracted.createdTime,
        updated_time: extracted.updatedTime,
        voteup_count: extracted.voteupCount,
        comment_count: extracted.commentCount
      });
      
      logger.info(`回答抓取并保存成功！文件路径: ${filePath}`);
      process.exitCode = 0;
    } catch (e: any) {
      logger.error(`抓取回答失败: ${e.message}`);
      process.exitCode = 1;
    } finally {
      await closeBrowserContext();
    }
  });

program
  .command('save-article <url>')
  .description('抓取并保存单个知乎文章')
  .option('-o, --output <dir>', '指定输出根目录', defaultOutputDir)
  .option('-d, --dry-run', '预览模式，不实际写入文件和数据库')
  .action(async (url, options) => {
    logger.info(`开始抓取文章: ${url}`);
    let context;
    try {
      context = await getBrowserContext(true);
      const page = await context.newPage();
      
      const extracted = await scrapeArticle(page, url);

      if (options.dryRun) {
        logger.info('=== [DRY RUN 预览] ===');
        logger.info(`类型: 文章`);
        logger.info(`标题: ${extracted.title}`);
        logger.info(`作者: ${extracted.authorName}`);
        logger.info(`发布时间: ${new Date(extracted.createdTime * 1000).toLocaleString()}`);
        logger.info(`赞同数: ${extracted.voteupCount}`);
        logger.info(`正文预览 (前 200 字):\n${extracted.contentMarkdown.slice(0, 200)}...`);
        logger.info('=======================');
        process.exitCode = 0;
        return;
      }

      const outputDir = path.resolve(process.cwd(), options.output);
      const filePath = await writeMarkdownFile(extracted, outputDir);

      saveItem({
        id: extracted.id,
        item_type: 'article',
        author_id: extracted.authorId,
        author_name: extracted.authorName,
        title: extracted.title,
        answer_id: null,
        question_id: null,
        article_id: extracted.articleId || null,
        url: extracted.url,
        question_url: null,
        created_time: extracted.createdTime,
        updated_time: extracted.updatedTime,
        voteup_count: extracted.voteupCount,
        comment_count: extracted.commentCount
      });
      
      logger.info(`文章抓取并保存成功！文件路径: ${filePath}`);
      process.exitCode = 0;
    } catch (e: any) {
      logger.error(`抓取文章失败: ${e.message}`);
      process.exitCode = 1;
    } finally {
      await closeBrowserContext();
    }
  });

const taskCmd = program.command('task').description('管理与运行批量备份任务');

taskCmd
  .command('create <peopleId>')
  .description('创建一个新的批量抓取任务')
  .option('-t, --types <type>', '要抓取的类型: answers | articles | all', 'all')
  .option('-n, --top-n <number>', '限制抓取前 N 条（按排序规则计算）', (val) => parseInt(val, 10), null)
  .option('-s, --sort <sort>', '排序规则: time (按发布时间) | vote (按点赞数)', 'time')
  .option('-o, --output <dir>', '指定输出目录', defaultOutputDir)
  .option('-d, --dry-run', '预览模式，不实际写入数据库和磁盘')
  .action(async (peopleId, options) => {
    try {
      if (options.dryRun) {
        logger.info(`=== [DRY RUN 任务创建预览] ===`);
        logger.info(`正在为答主 ${peopleId} 扫描列表索引...`);
        let context;
        try {
          context = await getBrowserContext(true);
          const page = await context.newPage();
          const scrapedIndexes = [];
          const scanLimit = scanLimitForSelection(options.topN, options.sort);
          if (options.types === 'answers' || options.types === 'all') {
            const answers = await scrapePeopleIndex(page, peopleId, 'answers', scanLimit);
            scrapedIndexes.push(...answers);
          }
          if (options.types === 'articles' || options.types === 'all') {
            const articles = await scrapePeopleIndex(page, peopleId, 'articles', scanLimit);
            scrapedIndexes.push(...articles);
          }

          logger.info('=== 扫描结果预览 (前 20 条) ===');
          logger.info(`总发现条目: ${scrapedIndexes.length}`);
          
          const selectedIndexes = selectIndexItems(scrapedIndexes, options.topN, options.sort);
          const previewItems = selectedIndexes.slice(0, 20);
          previewItems.forEach((item, index) => {
            const typeStr = item.type === 'answer' ? '回答' : '文章';
            const dateStr = new Date(item.createdTime * 1000).toLocaleDateString();
            logger.info(`[${index + 1}] [${typeStr}] ${item.title} (点赞: ${item.voteupCount} | 发布于: ${dateStr})`);
            logger.info(`    URL: ${item.url}`);
          });

          if (selectedIndexes.length > 20) {
            logger.info(`... 以及另外 ${selectedIndexes.length - 20} 个条目`);
          }
          logger.info('=======================');
          process.exitCode = 0;
          return;
        } catch (err: any) {
          logger.error(`Dry-run 扫描出错: ${err.message}`);
          process.exitCode = 1;
          return;
        } finally {
          await closeBrowserContext();
        }
      }

      const taskId = await createTask(peopleId, options.types, {
        topN: options.topN,
        sortBy: options.sort,
        outputDir: options.output
      });
      logger.info(`任务创建成功！任务ID: ${taskId}`);
      logger.info(`提示：可运行 "npx tsx src/cli.ts task start ${taskId}" 启动该任务`);
      process.exitCode = 0;
    } catch (e: any) {
      logger.error(`创建任务失败: ${e.message}`);
      process.exitCode = 1;
    }
  });

taskCmd
  .command('start <taskId>')
  .description('开始或恢复执行指定的备份任务')
  .option('-f, --force', '强制重新抓取所有条目（保留最后成功文档并重置任务进度）')
  .action(async (taskId, options) => {
    try {
      if (options.force) {
        logger.info(`正在强制重置任务 ${taskId} 的状态以进行重新爬取...`);
        resetTaskForce(taskId);

        logger.info('已保留最后成功的 Markdown；新结果验证成功后再发布为新 revision。');
      }

      logger.info(`准备启动任务: ${taskId}`);
      await runTask(taskId);
      process.exitCode = 0;
    } catch (e: any) {
      logger.error(`任务执行失败: ${e.message}`);
      process.exitCode = 1;
    }
  });

taskCmd
  .command('pause <taskId>')
  .description('暂停正在执行的任务')
  .action(async (taskId) => {
    try {
      saveTask({ id: taskId, status: 'paused' });
      logger.info(`已向任务 ${taskId} 发送暂停指令。将在当前单篇抓取完毕后生效。`);
      process.exitCode = 0;
    } catch (e: any) {
      logger.error(`暂停任务失败: ${e.message}`);
      process.exitCode = 1;
    }
  });

taskCmd
  .command('retry-failed')
  .description('重跑所有含失败条目的任务（如登录态过期导致中断，重新 login 后用此命令断点续跑）')
  .action(async () => {
    try {
      const candidates = getTasks().filter(
        t => t.status !== 'running' && (t.status === 'failed' || t.status === 'partial_success' || t.failed_count > 0)
      );
      if (candidates.length === 0) {
        console.log('没有需要重跑的失败任务。');
        process.exitCode = 0;
        return;
      }
      for (const t of candidates) {
        logger.info(`重跑任务 ${t.id}（作者: ${t.author_name || t.author_id} | 失败 ${t.failed_count} / 总数 ${t.total_count}）...`);
        await runTask(t.id);
      }
      process.exitCode = 0;
    } catch (e: any) {
      logger.error(`重跑失败任务出错: ${e.message}`);
      process.exitCode = 1;
    }
  });

taskCmd
  .command('list')
  .description('列出所有已创建的任务及其进度')
  .action(() => {
    try {
      const tasks = getTasks();
      if (tasks.length === 0) {
        console.log('当前没有任何任务。');
        process.exitCode = 0;
        return;
      }
      console.log('\n==================== 任务列表 ====================');
      for (const t of tasks) {
        const pct = t.total_count > 0 ? Math.floor(((t.success_count + t.failed_count) / t.total_count) * 100) : 0;
        console.log(`任务ID: ${t.id}`);
        console.log(`  目标答主: ${t.author_id} (${t.author_name || '未拉取到姓名'})`);
        console.log(`  内容类型: ${t.item_types} | 排序方式: ${t.sort_by} | 限制Top N: ${t.top_n || '无'}`);
        console.log(`  输出目录: ${t.output_dir}`);
        console.log(`  状态: ${t.status.toUpperCase()}`);
        console.log(`  进度: ${pct}% [成功: ${t.success_count} | 失败: ${t.failed_count} | 总数: ${t.total_count}]`);
        console.log('--------------------------------------------------');
      }
      process.exitCode = 0;
    } catch (e: any) {
      logger.error(`列出任务失败: ${e.message}`);
      process.exitCode = 1;
    }
  });

program
  .command('web')
  .description('启动 Web 数据大屏与控制台服务')
  .option('-p, --port <port>', '指定监听端口', '3000')
  .action((options) => {
    const port = parseInt(options.port, 10) || 3000;
    startServer(port);
  });

program.parse(process.argv);

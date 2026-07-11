import { DatabaseSync } from 'node:sqlite';
import * as path from 'path';

const db = new DatabaseSync(path.resolve(process.cwd(), 'zhihu-packer.db'));

console.log('--- tasks ---');
const tasks = db.prepare('SELECT * FROM tasks').all();
console.log(JSON.stringify(tasks, null, 2));

console.log('--- task_items ---');
const taskItems = db.prepare('SELECT * FROM task_items').all();
console.log(JSON.stringify(taskItems, null, 2));

console.log('--- items ---');
const items = db.prepare('SELECT * FROM items').all();
console.log(JSON.stringify(items, null, 2));

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { invokeCommand } from '../ipc';
import { addBookmark, getReaderLocator, listBookmarks, removeBookmark, saveReaderLocator } from './ipc';
import { makeLocator } from './locator';

vi.mock('../ipc', () => ({ invokeCommand: vi.fn(), invokeWithTimeout: vi.fn() }));
const invoke = vi.mocked(invokeCommand);
const locator = makeLocator('b1', 'c2', { kind: 'text', quote: '继续阅读', offset: 20 }, 0.42, '2026-10-18T12:00:00Z');
beforeEach(() => { invoke.mockReset(); vi.stubGlobal('localStorage', undefined); });

describe('reader IPC compatibility with registered Rust commands', () => {
  it('reads the JSON string actually returned by Rust', async () => {
    invoke.mockResolvedValue(JSON.stringify(locator));
    expect(await getReaderLocator('b1')).toEqual(locator);
    expect(invoke).toHaveBeenCalledWith('load_reader_locator', { bookId: 'b1' });
  });
  it('serializes positions and sequences writes even after failure', async () => {
    let finish!: () => void;
    invoke.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; })).mockResolvedValue(undefined);
    const first = saveReaderLocator('b1', locator);
    const second = saveReaderLocator('b1', { ...locator, ratio: 0.8 });
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    finish(); await first; await second;
    expect(invoke.mock.calls[1]).toEqual(['save_reader_locator', { bookId: 'b1', locatorJson: JSON.stringify({ ...locator, ratio: 0.8 }) }]);
    invoke.mockRejectedValueOnce(new Error('busy'));
    await expect(saveReaderLocator('b1', locator)).rejects.toThrow('busy');
    invoke.mockResolvedValue(undefined);
    await saveReaderLocator('b1', locator);
  });
  it('decodes bookmark rows and preserves their locators', async () => {
    invoke.mockResolvedValue([{ bookmarkId: 'bm1', locatorJson: JSON.stringify(locator), label: '这里', createdAt: locator.updated }]);
    expect((await listBookmarks('b1'))[0].locator).toEqual(locator);
    invoke.mockResolvedValue({ bookmarkId: 'bm2' });
    expect(await addBookmark('b1', locator, '这里')).toBe('bm2');
    await removeBookmark('b1', 'bm2');
    const rust = readFileSync(new URL('../../../src-tauri/src/lib.rs', import.meta.url), 'utf8');
    const registered = rust.split('.invoke_handler(tauri::generate_handler![')[1]?.split('])')[0] ?? '';
    for (const [command] of invoke.mock.calls) expect(registered).toMatch(new RegExp(`\\b${command}\\b`));
  });
});

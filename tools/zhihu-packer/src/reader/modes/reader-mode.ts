export interface SharedReadingState {
  schemaVersion: 1;
  current: string;
  position: number;
  read: string[];
  updated: string;
}

export type ReaderMode =
  | { kind: 'file' }
  | { kind: 'packed' }
  | {
      kind: 'served';
      contentBase: string;
      loadProgress: () => Promise<SharedReadingState>;
      saveProgress: (progress: SharedReadingState) => Promise<void>;
    };

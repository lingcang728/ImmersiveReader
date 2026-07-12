export type TrashItem = {
	readonly schemaVersion: number;
	readonly trashId: string;
	readonly bookId: string;
	readonly title: string;
	readonly originalRelativePath: string;
	readonly trashRelativePath: string;
	readonly deletedAt: string;
	readonly revision: number;
};

export type TrashDeleteResult = {
	readonly deletedItems: number;
	readonly releasedBytes: number;
};

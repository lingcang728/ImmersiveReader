import { renderMarkdownDocument } from './markdown';
import type { RenderedMarkdownDocument } from './markdown';

type RenderRequest = {
	id: number;
	source: string;
};

type RenderResponse =
	| {
			id: number;
			result: RenderedMarkdownDocument;
	  }
	| {
			id: number;
			error: string;
	  };

self.onmessage = async (event: MessageEvent<RenderRequest>) => {
	const { id, source } = event.data;
	try {
		const result = await renderMarkdownDocument(source);
		self.postMessage({ id, result } satisfies RenderResponse);
	} catch (err) {
		self.postMessage({
			id,
			error: err instanceof Error ? err.message : String(err)
		} satisfies RenderResponse);
	}
};

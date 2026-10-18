import { renderMarkdownDocument } from './markdown';
import type { MarkdownRenderOptions, RenderedMarkdownDocument } from './markdown';

type RenderRequest = {
	id: number;
	source: string;
	options?: MarkdownRenderOptions;
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
	const { id, source, options } = event.data;
	try {
		const result = await renderMarkdownDocument(source, options);
		self.postMessage({ id, result } satisfies RenderResponse);
	} catch (err) {
		self.postMessage({
			id,
			error: err instanceof Error ? err.message : String(err)
		} satisfies RenderResponse);
	}
};

let mermaidInstance: any = null;
let mermaidInitialized = false;

export async function getMermaid() {
  if (!mermaidInstance) {
    const module = await import('mermaid');
    mermaidInstance = module.default;
  }
  if (!mermaidInitialized) {
    mermaidInstance.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      theme: 'default',
    });
    mermaidInitialized = true;
  }
  return mermaidInstance;
}

export const hashText = (text: string) => {
  let hash = 0;
  for (let i = 0; i < text.length; i += 1) {
    hash = (hash * 31 + text.charCodeAt(i)) | 0;
  }
  return Math.abs(hash).toString(36);
};

export function normalizeMarkdownish(text: string): string {
  return text
    .replace(/\s+(#{1,6}\s)/g, '\n\n$1')
    .replace(/\s+(\d+\.\s)/g, '\n$1')
    .replace(/\s+(-\s)/g, '\n$1')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

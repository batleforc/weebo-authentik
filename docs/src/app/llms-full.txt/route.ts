import { getLLMText, source } from '@/lib/source';

export const revalidate = false;
// Emit as a static file under `output: 'export'`.
export const dynamic = 'force-static';

export async function GET() {
  const scan = source.getPages().map(getLLMText);
  const scanned = await Promise.all(scan);

  return new Response(scanned.join('\n\n'));
}

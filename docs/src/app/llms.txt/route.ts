import { source } from '@/lib/source';
import { llms } from 'fumadocs-core/source';

export const revalidate = false;
// Emit as a static file under `output: 'export'`.
export const dynamic = 'force-static';

export function GET() {
  return new Response(llms(source).index());
}

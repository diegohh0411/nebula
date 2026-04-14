export async function startViewTransition(callback: () => void | Promise<void>): Promise<void> {
  if (!(document as any).startViewTransition) {
    await callback();
    return;
  }
  return (document as any).startViewTransition(callback).finished;
}

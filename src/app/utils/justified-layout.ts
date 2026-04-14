import { Image, SearchResult } from '../models/models';

export interface JustifiedRow {
  images: (Image | SearchResult)[];
  rowHeight: number;
}

/**
 * Calculates a justified layout for a set of images.
 * @param images The list of images to layout.
 * @param containerWidth The width of the gallery container.
 * @param targetRowHeight The desired height for each row.
 * @param gap The gap between images in pixels.
 */
export function buildJustifiedRows(
  images: (Image | SearchResult)[],
  containerWidth: number,
  targetRowHeight: number,
  gap: number = 4
): JustifiedRow[] {
  const rows: JustifiedRow[] = [];
  let currentRow: (Image | SearchResult)[] = [];
  let currentRowWidth = 0;

  for (const img of images) {
    const id = 'id' in img ? img.id : img.image_id;
    // Stable pseudo-random aspect ratio between 0.8 and 1.8
    const aspectRatio = 0.8 + ((id * 54321) % 1000) / 1000;

    const imgWidth = targetRowHeight * aspectRatio;

    if (currentRowWidth + imgWidth + gap * currentRow.length > containerWidth) {
      // Current row is full, calculate its final height to fit the container
      const totalRatios = currentRow.map(i => {
        const iId = 'id' in i ? i.id : i.image_id;
        return 0.8 + ((iId * 54321) % 1000) / 1000;
      }).reduce((a, b) => a + b, 0);
      const availableWidth = containerWidth - (gap * (currentRow.length - 1));
      const rowHeight = availableWidth / totalRatios;

      rows.push({ images: currentRow, rowHeight });

      currentRow = [img];
      currentRowWidth = imgWidth;
    } else {
      currentRow.push(img);
      currentRowWidth += imgWidth;
    }
  }


  // Handle the last (incomplete) row
  if (currentRow.length > 0) {
    // For the last row, we don't stretch it to full width (it looks bad)
    // We just keep the target height.
    rows.push({ images: currentRow, rowHeight: targetRowHeight });
  }

  return rows;
}

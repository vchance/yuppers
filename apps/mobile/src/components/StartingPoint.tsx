import type { Template } from '@yuppers/shared';

import { useI18n } from '../lib/context';
import { Actions, Button, Heading, Hint, Notice, P } from './ui';

/**
 * What the common agreement a draft was started from has to say about
 * itself, above the items (DESIGN.md §4.4): the hint, the note that the grey
 * text is only an example, and, for the job, the warning about New Jersey.
 * It stays until the first edit, then folds away behind a link. The warning
 * does not fold: it is a statement about the law, not about the form.
 */
export function StartingPoint({
  template,
  open,
  onToggle,
}: {
  template: Template;
  open: boolean;
  onToggle(): void;
}) {
  const { wording } = useI18n();
  const w = wording.templates;
  const entry = w.entries[template.id];
  return (
    <>
      {entry.warning && <Notice tone="warning">{entry.warning}</Notice>}
      {open ? (
        <Notice quiet>
          <Heading level={2}>{w.bandHeading}</Heading>
          <P>{entry.hint}</P>
          <P>{w.bandExamples}</P>
          <Actions>
            <Button label={w.bandHide} expanded onPress={onToggle} />
          </Actions>
        </Notice>
      ) : (
        <Actions>
          <Button variant="link" label={w.bandShow} expanded={false} onPress={onToggle} />
        </Actions>
      )}
    </>
  );
}

/**
 * Turns every item's provider round in one tap, for when the other person is
 * the one writing: each common agreement is written from one side.
 */
export function SwapSides({ onSwap }: { onSwap(): void }) {
  const { wording } = useI18n();
  const w = wording.templates;
  return (
    <>
      <Actions>
        <Button label={w.swapSides} hint={w.swapSidesHint} onPress={onSwap} />
      </Actions>
      <Hint>{w.swapSidesHint}</Hint>
    </>
  );
}

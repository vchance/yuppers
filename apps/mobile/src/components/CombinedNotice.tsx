import { useState } from 'react';

import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';
import { Actions, Button, Card, P } from './ui';

/**
 * The notice that another account was combined into this one, shown once
 * where neither account had an email address to tell, as on the web
 * (`apps/web/src/components/CombinedNotice.tsx`). Nothing is texted.
 */
export function CombinedNotice() {
  const { wording, fmt, moment } = useI18n();
  const { account, setAccount } = useSession();
  const [busy, setBusy] = useState(false);
  const at = account?.combined_notice;
  if (!at) return null;
  const w = wording.combine;
  return (
    <Card>
      <P>{fmt(w.noticeBanner, { date: moment(at) })}</P>
      <Actions>
        <Button
          label={w.noticeDismiss}
          disabled={busy}
          onPress={() => {
            setBusy(true);
            api.updateMe({ dismiss_combined_notice: true }).then(setAccount, () => setBusy(false));
          }}
        />
      </Actions>
    </Card>
  );
}

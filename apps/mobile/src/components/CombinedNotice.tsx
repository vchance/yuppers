import { noticeText } from '@yuppers/shared';
import { useState } from 'react';

import { useI18n, useSession } from '../lib/context';
import { api } from '../lib/session';
import { Actions, Button, Card, P } from './ui';

/**
 * A notice about the account shown once where there was no email address to
 * tell, as on the web (`apps/web/src/components/CombinedNotice.tsx`):
 * another account combined into this one, or its phone number replaced or
 * removed. Nothing is texted.
 */
export function CombinedNotice() {
  const { wording, moment, language } = useI18n();
  const { account, setAccount } = useSession();
  const [busy, setBusy] = useState(false);
  const notice = account?.notice;
  if (!notice) return null;
  return (
    <Card>
      <P>{noticeText(wording, notice.kind, moment(notice.at), language)}</P>
      <Actions>
        <Button
          label={wording.combine.noticeDismiss}
          disabled={busy}
          onPress={() => {
            setBusy(true);
            api.updateMe({ dismiss_notice: true }).then(setAccount, () => setBusy(false));
          }}
        />
      </Actions>
    </Card>
  );
}

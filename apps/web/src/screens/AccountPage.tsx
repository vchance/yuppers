import { formatPhone } from '@yuppers/shared'
import { useState } from 'react'

import { useI18n, useSession } from '../app/context'
import { navigate } from '../app/router'
import { paths } from '../app/routes'
import { Appearance } from '../components/Appearance'
import { BuildVersion } from '../components/BuildVersion'
import { LegalLink } from '../components/LegalLink'
import { PaymentHandles } from '../components/PaymentHandles'
import { PageHeading } from '../components/ui'
import { api } from '../lib/api'
import { BlockedPeople } from './BlockedPeople'
import DeleteAccount from './DeleteAccount'
import { ProfileForm } from './ProfileForm'

/**
 * The account: what it is verified with, its name and language, its
 * payment options, this device's appearance, and signing out.
 */
export default function AccountPage() {
  const { wording } = useI18n()
  const { account, setAccount } = useSession()
  const w = wording.profile
  const [leaving, setLeaving] = useState(false)

  if (!account) return null

  async function signOut() {
    setLeaving(true)
    try {
      await api.signOut()
    } catch {
      // Whether or not the service heard, this browser stops acting as the
      // account: a refusal means the session had already ended.
    }
    setAccount(null)
    navigate(paths.home)
  }

  return (
    <>
      <PageHeading>{w.title}</PageHeading>
      <dl>
        {account.email && (
          <>
            <dt>{w.emailLabel}</dt>
            <dd>{account.email}</dd>
          </>
        )}
        {account.phone && (
          <>
            <dt>{w.phoneLabel}</dt>
            <dd dir="ltr">{formatPhone(account.phone)}</dd>
          </>
        )}
      </dl>
      <ProfileForm account={account} first={false} />
      <PaymentHandles />
      <Appearance />
      <BlockedPeople />
      <hr />
      <div className="actions">
        <button type="button" disabled={leaving} onClick={signOut}>
          {wording.nav.signOut}
        </button>
      </div>
      <DeleteAccount account={account} />
      <p className="learn-more legal-links">
        <LegalLink document="privacy" />
        <LegalLink document="terms" />
      </p>
      <BuildVersion />
    </>
  )
}

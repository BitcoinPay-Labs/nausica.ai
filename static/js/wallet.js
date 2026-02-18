// Wallet State
let walletState = {
    network: localStorage.getItem('bsv_network') || 'mainnet',
    wif: localStorage.getItem('bsv_wif') || null,
    address: localStorage.getItem('bsv_address') || null,
    balance: 0
};

// Initialize wallet on page load
document.addEventListener('DOMContentLoaded', function() {
    initWallet();
});

function initWallet() {
    // Update network badge
    updateNetworkBadge();
    
    // Check if logged in
    if (walletState.wif && walletState.address) {
        showWalletDashboard();
        refreshBalance();
    }
    
    // Re-create icons for wallet modal
    if (typeof lucide !== 'undefined') {
        lucide.createIcons();
    }
}

// Modal functions
function openWalletModal() {
    document.getElementById('walletModalOverlay').classList.add('visible');
    document.getElementById('walletModal').classList.add('visible');
    
    // Update network buttons
    updateNetworkButtons();
    
    // Re-create icons
    if (typeof lucide !== 'undefined') {
        lucide.createIcons();
    }
}

function closeWalletModal() {
    document.getElementById('walletModalOverlay').classList.remove('visible');
    document.getElementById('walletModal').classList.remove('visible');
    clearWalletStatus();
}

// Network switching
function switchNetwork(network) {
    walletState.network = network;
    localStorage.setItem('bsv_network', network);
    
    updateNetworkBadge();
    updateNetworkButtons();
    
    // If logged in, refresh balance for new network
    if (walletState.address) {
        refreshBalance();
    }
    
    showWalletStatus(`Switched to ${network}`, 'success');
}

function updateNetworkBadge() {
    const badge = document.getElementById('networkBadge');
    if (badge) {
        badge.textContent = walletState.network === 'mainnet' ? 'Mainnet' : 'Testnet';
        badge.classList.toggle('testnet', walletState.network === 'testnet');
    }
}

function updateNetworkButtons() {
    const mainnetBtn = document.getElementById('mainnetBtn');
    const testnetBtn = document.getElementById('testnetBtn');
    
    if (mainnetBtn && testnetBtn) {
        mainnetBtn.classList.toggle('active', walletState.network === 'mainnet');
        testnetBtn.classList.toggle('active', walletState.network === 'testnet');
        testnetBtn.classList.toggle('testnet', walletState.network === 'testnet');
    }
}

// Tab switching
function showWalletTab(tab) {
    // Update tab buttons
    document.querySelectorAll('.wallet-tab').forEach(btn => {
        btn.classList.remove('active');
    });
    event.target.classList.add('active');
    
    // Show/hide tab content
    document.getElementById('createTab').classList.toggle('hidden', tab !== 'create');
    document.getElementById('wifTab').classList.toggle('hidden', tab !== 'wif');
    document.getElementById('mnemonicTab').classList.toggle('hidden', tab !== 'mnemonic');
    
    clearWalletStatus();
}

// Wallet creation
async function generateWallet() {
    showWalletStatus('Generating wallet...', 'loading');
    
    try {
        const response = await fetch('/api/wallet/generate', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ network: walletState.network })
        });
        
        const data = await response.json();
        
        if (data.success) {
            // Save wallet
            walletState.wif = data.wif;
            walletState.address = data.address;
            localStorage.setItem('bsv_wif', data.wif);
            localStorage.setItem('bsv_address', data.address);
            
            showWalletStatus('Wallet created! Save your WIF securely.', 'success');
            
            // Show WIF to user (important for backup)
            alert(`IMPORTANT: Save your WIF private key securely!\n\nWIF: ${data.wif}\n\nAddress: ${data.address}\n\nThis is the only time you will see this. If you lose it, you lose access to your funds.`);
            
            showWalletDashboard();
            refreshBalance();
        } else {
            showWalletStatus(data.error || 'Failed to generate wallet', 'error');
        }
    } catch (error) {
        showWalletStatus('Network error: ' + error.message, 'error');
    }
}

// WIF import
async function importWif() {
    const wif = document.getElementById('wifInput').value.trim();
    
    if (!wif) {
        showWalletStatus('Please enter a WIF', 'error');
        return;
    }
    
    showWalletStatus('Importing wallet...', 'loading');
    
    try {
        const response = await fetch('/api/wallet/import', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ wif: wif, network: walletState.network })
        });
        
        const data = await response.json();
        
        if (data.success) {
            walletState.wif = data.wif;
            walletState.address = data.address;
            localStorage.setItem('bsv_wif', data.wif);
            localStorage.setItem('bsv_address', data.address);
            
            document.getElementById('wifInput').value = '';
            showWalletStatus('Wallet imported successfully!', 'success');
            
            showWalletDashboard();
            refreshBalance();
        } else {
            showWalletStatus(data.error || 'Failed to import wallet', 'error');
        }
    } catch (error) {
        showWalletStatus('Network error: ' + error.message, 'error');
    }
}

// Mnemonic import (placeholder - needs BIP39 implementation)
async function importMnemonic() {
    const mnemonic = document.getElementById('mnemonicInput').value.trim();
    
    if (!mnemonic) {
        showWalletStatus('Please enter a mnemonic phrase', 'error');
        return;
    }
    
    // For now, show a message that mnemonic import is not yet implemented
    showWalletStatus('Mnemonic import coming soon. Please use WIF for now.', 'error');
}

// Show wallet dashboard
function showWalletDashboard() {
    document.getElementById('walletLogin').classList.add('hidden');
    document.getElementById('walletDashboard').classList.remove('hidden');
    
    // Update address display
    document.getElementById('walletAddress').textContent = walletState.address || '-';
    
    // Re-create icons
    if (typeof lucide !== 'undefined') {
        lucide.createIcons();
    }
}

// Show wallet login
function showWalletLogin() {
    document.getElementById('walletLogin').classList.remove('hidden');
    document.getElementById('walletDashboard').classList.add('hidden');
}

// Refresh balance
async function refreshBalance() {
    if (!walletState.address) return;
    
    showWalletStatus('Fetching balance...', 'loading');
    
    try {
        const response = await fetch('/api/wallet/balance', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ 
                address: walletState.address,
                network: walletState.network 
            })
        });
        
        const data = await response.json();
        
        if (data.success) {
            walletState.balance = data.balance;
            document.getElementById('balanceAmount').textContent = data.balance_bsv + ' BSV';
            document.getElementById('balanceSats').textContent = data.balance.toLocaleString() + ' sats';
            clearWalletStatus();
        } else {
            showWalletStatus(data.error || 'Failed to fetch balance', 'error');
        }
    } catch (error) {
        showWalletStatus('Network error: ' + error.message, 'error');
    }
}

// Copy wallet address
function copyWalletAddress() {
    if (walletState.address) {
        navigator.clipboard.writeText(walletState.address).then(() => {
            showWalletStatus('Address copied!', 'success');
            setTimeout(clearWalletStatus, 2000);
        });
    }
}

// Send form
function showSendForm() {
    document.getElementById('sendForm').classList.remove('hidden');
}

function hideSendForm() {
    document.getElementById('sendForm').classList.add('hidden');
    document.getElementById('sendToAddress').value = '';
    document.getElementById('sendAmount').value = '';
}

// Send BSV
async function sendBsv() {
    const toAddress = document.getElementById('sendToAddress').value.trim();
    const amount = parseInt(document.getElementById('sendAmount').value);
    
    if (!toAddress) {
        showWalletStatus('Please enter recipient address', 'error');
        return;
    }
    
    if (!amount || amount <= 0) {
        showWalletStatus('Please enter a valid amount', 'error');
        return;
    }
    
    if (amount > walletState.balance) {
        showWalletStatus('Insufficient balance', 'error');
        return;
    }
    
    // Confirm send
    if (!confirm(`Send ${amount} satoshis to ${toAddress}?`)) {
        return;
    }
    
    showWalletStatus('Sending...', 'loading');
    
    try {
        const response = await fetch('/api/wallet/send', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                wif: walletState.wif,
                to_address: toAddress,
                amount_satoshis: amount,
                network: walletState.network
            })
        });
        
        const data = await response.json();
        
        if (data.success) {
            showWalletStatus(`Sent! TXID: ${data.txid.substring(0, 16)}...`, 'success');
            hideSendForm();
            refreshBalance();
        } else {
            showWalletStatus(data.error || 'Failed to send', 'error');
        }
    } catch (error) {
        showWalletStatus('Network error: ' + error.message, 'error');
    }
}

// Logout
function logoutWallet() {
    if (!confirm('Are you sure you want to logout? Make sure you have saved your WIF!')) {
        return;
    }
    
    walletState.wif = null;
    walletState.address = null;
    walletState.balance = 0;
    
    localStorage.removeItem('bsv_wif');
    localStorage.removeItem('bsv_address');
    
    showWalletLogin();
    showWalletStatus('Logged out', 'success');
}

// Status messages
function showWalletStatus(message, type) {
    const status = document.getElementById('walletStatus');
    if (status) {
        status.textContent = message;
        status.className = 'wallet-status ' + type;
    }
}

function clearWalletStatus() {
    const status = document.getElementById('walletStatus');
    if (status) {
        status.textContent = '';
        status.className = 'wallet-status';
    }
}
